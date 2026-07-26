use std::slice;

use windows::core::Interface;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Resource, ID3D11Texture2D,
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE,
    D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Dxgi::{IDXGIAdapter, IDXGIAdapter1};

use crate::infrastructure::screenshot::model::{MonitorDescriptor, MonitorFrame};
use crate::infrastructure::screenshot::ScreenshotError;

#[derive(Clone)]
pub(super) struct DeviceResources {
    pub(super) device: ID3D11Device,
    pub(super) context: ID3D11DeviceContext,
}

impl DeviceResources {
    pub(super) fn for_adapter(adapter: &IDXGIAdapter1) -> Result<Self, ScreenshotError> {
        let adapter: IDXGIAdapter = adapter
            .cast()
            .map_err(|error| d3d_error("IDXGIAdapter cast", error))?;
        create_device(Some(&adapter), D3D_DRIVER_TYPE_UNKNOWN)
    }

    pub(super) fn default_hardware() -> Result<Self, ScreenshotError> {
        create_device(None, D3D_DRIVER_TYPE_HARDWARE)
    }
}

fn create_device(
    adapter: Option<&IDXGIAdapter>,
    driver_type: windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE,
) -> Result<DeviceResources, ScreenshotError> {
    let mut device = None;
    let mut context = None;
    unsafe {
        D3D11CreateDevice(
            adapter,
            driver_type,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&[D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    }
    .map_err(|error| d3d_error("D3D11CreateDevice", error))?;
    Ok(DeviceResources {
        device: device.ok_or_else(|| {
            ScreenshotError::BackendUnavailable("D3D11CreateDevice returned no device".to_owned())
        })?,
        context: context.ok_or_else(|| {
            ScreenshotError::BackendUnavailable(
                "D3D11CreateDevice returned no immediate context".to_owned(),
            )
        })?,
    })
}

pub(super) fn read_bgra_texture(
    backend: &str,
    monitor: &MonitorDescriptor,
    resources: &DeviceResources,
    source_texture: &ID3D11Texture2D,
) -> Result<MonitorFrame, ScreenshotError> {
    let mut source_desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { source_texture.GetDesc(&mut source_desc) };
    let expected_width = monitor
        .physical_bounds
        .width()
        .ok_or(ScreenshotError::InvalidTopology)?;
    let expected_height = monitor
        .physical_bounds
        .height()
        .ok_or(ScreenshotError::InvalidTopology)?;
    if source_desc.Width != expected_width
        || source_desc.Height != expected_height
        || source_desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM
    {
        return Err(ScreenshotError::BackendUnavailable(format!(
            "{backend} frame contract mismatch for {}: frame={}x{} format={:?}, expected={}x{} BGRA8",
            monitor.id,
            source_desc.Width,
            source_desc.Height,
            source_desc.Format,
            expected_width,
            expected_height
        )));
    }

    let staging_desc = D3D11_TEXTURE2D_DESC {
        Width: source_desc.Width,
        Height: source_desc.Height,
        MipLevels: 1,
        ArraySize: 1,
        Format: source_desc.Format,
        SampleDesc: source_desc.SampleDesc,
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    };
    let mut staging_texture = None;
    unsafe {
        resources
            .device
            .CreateTexture2D(&staging_desc, None, Some(&mut staging_texture))
    }
    .map_err(|error| d3d_error("CreateTexture2D(staging)", error))?;
    let staging_texture = staging_texture.ok_or_else(|| {
        ScreenshotError::BackendUnavailable(
            "CreateTexture2D returned no staging texture".to_owned(),
        )
    })?;
    let staging_resource: ID3D11Resource = staging_texture
        .cast()
        .map_err(|error| d3d_error("staging resource cast", error))?;
    let source_resource: ID3D11Resource = source_texture
        .cast()
        .map_err(|error| d3d_error("source texture resource cast", error))?;
    unsafe {
        resources
            .context
            .CopyResource(&staging_resource, &source_resource);
        resources.context.Flush();
    }

    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe {
        resources
            .context
            .Map(&staging_resource, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
    }
    .map_err(|error| d3d_error("ID3D11DeviceContext::Map", error))?;
    let _map_guard = MappedResource::new(&resources.context, &staging_resource);
    let bgra = copy_mapped_bgra(
        mapped.pData.cast_const().cast(),
        mapped.RowPitch,
        source_desc.Width,
        source_desc.Height,
    )?;
    if bgra
        .chunks_exact(4)
        .all(|pixel| pixel[0] == 0 && pixel[1] == 0 && pixel[2] == 0)
    {
        return Err(ScreenshotError::BackendUnavailable(format!(
            "{backend} returned an all-black frame for {}",
            monitor.id
        )));
    }
    let stride = usize::try_from(source_desc.Width)
        .map_err(|_| ScreenshotError::ArithmeticOverflow)?
        .checked_mul(4)
        .ok_or(ScreenshotError::ArithmeticOverflow)?;
    let frame = MonitorFrame {
        monitor: monitor.clone(),
        width: source_desc.Width,
        height: source_desc.Height,
        stride,
        bgra,
    };
    frame.validate()?;
    Ok(frame)
}

fn copy_mapped_bgra(
    source: *const u8,
    row_pitch: u32,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, ScreenshotError> {
    if source.is_null() {
        return Err(ScreenshotError::InvalidFrame);
    }
    let row_bytes = usize::try_from(width)
        .map_err(|_| ScreenshotError::ArithmeticOverflow)?
        .checked_mul(4)
        .ok_or(ScreenshotError::ArithmeticOverflow)?;
    let source_stride =
        usize::try_from(row_pitch).map_err(|_| ScreenshotError::ArithmeticOverflow)?;
    if source_stride < row_bytes {
        return Err(ScreenshotError::InvalidFrame);
    }
    let height = usize::try_from(height).map_err(|_| ScreenshotError::ArithmeticOverflow)?;
    let byte_len = row_bytes
        .checked_mul(height)
        .ok_or(ScreenshotError::ArithmeticOverflow)?;
    let mut bgra = Vec::with_capacity(byte_len);
    for row in 0..height {
        let offset = row
            .checked_mul(source_stride)
            .ok_or(ScreenshotError::ArithmeticOverflow)?;
        let source_row = unsafe { slice::from_raw_parts(source.add(offset), row_bytes) };
        bgra.extend_from_slice(source_row);
    }
    for alpha in bgra.iter_mut().skip(3).step_by(4) {
        *alpha = 255;
    }
    Ok(bgra)
}

pub(super) fn d3d_error(stage: &str, error: windows::core::Error) -> ScreenshotError {
    ScreenshotError::BackendUnavailable(format!("D3D11 {stage} failed: {error}"))
}

struct MappedResource<'a> {
    context: &'a ID3D11DeviceContext,
    resource: &'a ID3D11Resource,
}

impl<'a> MappedResource<'a> {
    fn new(context: &'a ID3D11DeviceContext, resource: &'a ID3D11Resource) -> Self {
        Self { context, resource }
    }
}

impl Drop for MappedResource<'_> {
    fn drop(&mut self) {
        unsafe { self.context.Unmap(self.resource, 0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapped_rows_drop_gpu_padding_and_normalize_alpha() {
        let source = [
            1, 2, 3, 4, 5, 6, 7, 8, 90, 91, 92, 93, 10, 11, 12, 13, 14, 15, 16, 17, 94, 95, 96, 97,
        ];
        let copied = copy_mapped_bgra(source.as_ptr(), 12, 2, 2).unwrap();
        assert_eq!(
            copied,
            vec![1, 2, 3, 255, 5, 6, 7, 255, 10, 11, 12, 255, 14, 15, 16, 255]
        );
    }

    #[test]
    fn mapped_rows_reject_short_pitch() {
        let source = [0u8; 8];
        assert_eq!(
            copy_mapped_bgra(source.as_ptr(), 7, 2, 1),
            Err(ScreenshotError::InvalidFrame)
        );
    }
}
