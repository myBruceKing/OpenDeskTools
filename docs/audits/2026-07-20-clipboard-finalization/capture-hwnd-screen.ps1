[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [Int64]$TargetHwnd,

    [Parameter(Mandatory = $true)]
    [string]$OutputPath,

    [ValidateRange(0, 4096)]
    [int]$MarginLeft = 0,

    [ValidateRange(0, 4096)]
    [int]$MarginTop = 0,

    [ValidateRange(0, 4096)]
    [int]$MarginRight = 0,

    [ValidateRange(0, 4096)]
    [int]$MarginBottom = 0,

    [switch]$Force
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$resolvedOutput = [IO.Path]::GetFullPath($OutputPath)
if ([IO.Path]::GetExtension($resolvedOutput) -ne ".png") {
    throw "OutputPath must end in .png"
}
if ([IO.File]::Exists($resolvedOutput) -and -not $Force) {
    throw "OutputPath already exists"
}
[IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($resolvedOutput)) | Out-Null

Add-Type -AssemblyName System.Drawing
if (-not ("OpenDeskTools.FinalQa.NativeCapture" -as [type])) {
    Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;

namespace OpenDeskTools.FinalQa {
    [StructLayout(LayoutKind.Sequential)]
    public struct Rect { public int Left, Top, Right, Bottom; }

    public static class NativeCapture {
        [DllImport("user32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool IsWindow(IntPtr window);

        [DllImport("user32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool IsWindowVisible(IntPtr window);

        [DllImport("user32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool GetWindowRect(IntPtr window, out Rect rect);

        [DllImport("user32.dll")]
        public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    }
}
"@
}

$window = [IntPtr]$TargetHwnd
if (-not [OpenDeskTools.FinalQa.NativeCapture]::IsWindow($window)) {
    throw "Target HWND is not valid"
}
if (-not [OpenDeskTools.FinalQa.NativeCapture]::IsWindowVisible($window)) {
    throw "Target HWND is not visible"
}
$rect = New-Object OpenDeskTools.FinalQa.Rect
if (-not [OpenDeskTools.FinalQa.NativeCapture]::GetWindowRect($window, [ref]$rect)) {
    throw "GetWindowRect failed"
}
$captureLeft = $rect.Left - $MarginLeft
$captureTop = $rect.Top - $MarginTop
$width = $rect.Right - $rect.Left + $MarginLeft + $MarginRight
$height = $rect.Bottom - $rect.Top + $MarginTop + $MarginBottom
if ($width -le 0 -or $height -le 0) {
    throw "Target HWND has invalid dimensions"
}

$bitmap = New-Object Drawing.Bitmap($width, $height, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
$graphics = [Drawing.Graphics]::FromImage($bitmap)
try {
    $graphics.CopyFromScreen($captureLeft, $captureTop, 0, 0, [Drawing.Size]::new($width, $height), [Drawing.CopyPixelOperation]::SourceCopy)
    $bitmap.Save($resolvedOutput, [Drawing.Imaging.ImageFormat]::Png)
}
finally {
    $graphics.Dispose()
    $bitmap.Dispose()
}

[uint32]$ownerProcessId = 0
[void][OpenDeskTools.FinalQa.NativeCapture]::GetWindowThreadProcessId($window, [ref]$ownerProcessId)
[pscustomobject]@{
    hwnd = ("0x{0:X}" -f $window.ToInt64())
    processId = $ownerProcessId
    method = "Graphics.CopyFromScreen"
    windowBounds = [pscustomobject]@{ left = $rect.Left; top = $rect.Top; right = $rect.Right; bottom = $rect.Bottom }
    captureBounds = [pscustomobject]@{ left = $captureLeft; top = $captureTop; right = $captureLeft + $width; bottom = $captureTop + $height }
    output = $resolvedOutput
} | ConvertTo-Json -Depth 3
