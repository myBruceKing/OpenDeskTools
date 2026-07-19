[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateRange(1, [int]::MaxValue)]
    [int]$TargetProcessId,

    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$OutputPath,

    [ValidateSet("Auto", "PrintWindow", "Screen")]
    [string]$Mode = "Auto",

    [switch]$Force
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

Add-Type -AssemblyName System.Drawing

$resolvedOutputPath = [IO.Path]::GetFullPath($OutputPath)
if ([IO.Path]::GetExtension($resolvedOutputPath) -ne ".png") {
    throw "OutputPath must use the .png extension."
}
$outputDirectory = [IO.Path]::GetDirectoryName($resolvedOutputPath)
if ([string]::IsNullOrWhiteSpace($outputDirectory)) {
    throw "OutputPath must include a directory."
}
if ([IO.File]::Exists($resolvedOutputPath) -and -not $Force) {
    throw "OutputPath already exists. Pass -Force to replace it."
}

if (-not ("OpenDeskTools.VisualQa.NativeWindow" -as [type])) {
    Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;

namespace OpenDeskTools.VisualQa
{
    [StructLayout(LayoutKind.Sequential)]
    public struct Rect
    {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    public static class NativeWindow
    {
        public const int DwmExtendedFrameBounds = 9;
        public const int ShowRestore = 9;
        public const uint PrintWindowEntireWindow = 0;

        [DllImport("user32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool PrintWindow(IntPtr window, IntPtr targetDc, uint flags);

        [DllImport("user32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool GetWindowRect(IntPtr window, out Rect rect);

        [DllImport("dwmapi.dll")]
        public static extern int DwmGetWindowAttribute(
            IntPtr window,
            int attribute,
            out Rect value,
            int valueSize);

        [DllImport("user32.dll")]
        public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);

        [DllImport("user32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool IsWindow(IntPtr window);

        [DllImport("user32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool IsWindowVisible(IntPtr window);

        [DllImport("user32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool ShowWindow(IntPtr window, int command);

        [DllImport("user32.dll")]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool SetForegroundWindow(IntPtr window);

        [DllImport("user32.dll")]
        public static extern IntPtr GetForegroundWindow();
    }
}
"@
}

function Assert-TargetWindowIdentity {
    param(
        [int]$ProcessId,
        [IntPtr]$Window
    )

    $target = Get-Process -Id $ProcessId -ErrorAction Stop
    $target.Refresh()
    if ($target.MainWindowHandle -eq [IntPtr]::Zero) {
        throw "Process $ProcessId does not expose a main window."
    }
    if ($target.MainWindowHandle -ne $Window) {
        throw "Process $ProcessId main window changed before capture completed."
    }
    if (-not [OpenDeskTools.VisualQa.NativeWindow]::IsWindow($window)) {
        throw "Process $ProcessId returned a stale window handle."
    }
    if (-not [OpenDeskTools.VisualQa.NativeWindow]::IsWindowVisible($window)) {
        throw "Process $ProcessId main window is not visible."
    }

    [uint32]$ownerProcessId = 0
    [void][OpenDeskTools.VisualQa.NativeWindow]::GetWindowThreadProcessId(
        $window,
        [ref]$ownerProcessId
    )
    if ($ownerProcessId -ne [uint32]$ProcessId) {
        throw "Window ownership changed before capture."
    }

    return $target
}

function Get-TargetWindow {
    param([int]$ProcessId)

    $target = Get-Process -Id $ProcessId -ErrorAction Stop
    $target.Refresh()
    $window = $target.MainWindowHandle
    $target = Assert-TargetWindowIdentity -ProcessId $ProcessId -Window $window

    [pscustomobject]@{
        ProcessId = $ProcessId
        Executable = $target.Path
        Title = $target.MainWindowTitle
        Handle = $window
    }
}

function Get-CaptureRect {
    param([IntPtr]$Window)

    $rect = New-Object OpenDeskTools.VisualQa.Rect
    $rectSize = [Runtime.InteropServices.Marshal]::SizeOf(
        [type][OpenDeskTools.VisualQa.Rect]
    )
    $dwmResult = [OpenDeskTools.VisualQa.NativeWindow]::DwmGetWindowAttribute(
        $Window,
        [OpenDeskTools.VisualQa.NativeWindow]::DwmExtendedFrameBounds,
        [ref]$rect,
        $rectSize
    )
    if ($dwmResult -ne 0) {
        if (-not [OpenDeskTools.VisualQa.NativeWindow]::GetWindowRect($Window, [ref]$rect)) {
            throw "Windows could not resolve the target window bounds."
        }
    }

    $width = $rect.Right - $rect.Left
    $height = $rect.Bottom - $rect.Top
    if ($width -le 0 -or $height -le 0 -or $width -gt 32768 -or $height -gt 32768) {
        throw "Target window bounds are outside the supported capture range."
    }

    [pscustomobject]@{
        Left = $rect.Left
        Top = $rect.Top
        Width = $width
        Height = $height
    }
}

function Test-ImageHasVisibleDetail {
    param([System.Drawing.Bitmap]$Bitmap)

    $colors = [Collections.Generic.HashSet[int]]::new()
    $steps = 12
    for ($row = 0; $row -lt $steps; $row++) {
        $y = [Math]::Min(
            $Bitmap.Height - 1,
            [Math]::Max(0, [int](($row + 0.5) * $Bitmap.Height / $steps))
        )
        for ($column = 0; $column -lt $steps; $column++) {
            $x = [Math]::Min(
                $Bitmap.Width - 1,
                [Math]::Max(0, [int](($column + 0.5) * $Bitmap.Width / $steps))
            )
            [void]$colors.Add($Bitmap.GetPixel($x, $y).ToArgb())
            if ($colors.Count -ge 5) {
                return $true
            }
        }
    }
    return $false
}

function Invoke-PrintWindowCapture {
    param(
        [IntPtr]$Window,
        [pscustomobject]$Bounds,
        [int]$ProcessId
    )

    [void](Assert-TargetWindowIdentity -ProcessId $ProcessId -Window $Window)
    $bitmap = $null
    $graphics = $null
    $targetDc = [IntPtr]::Zero
    try {
        try {
            $bitmap = [Drawing.Bitmap]::new(
                $Bounds.Width,
                $Bounds.Height,
                [Drawing.Imaging.PixelFormat]::Format32bppArgb
            )
            $graphics = [Drawing.Graphics]::FromImage($bitmap)
            $graphics.Clear([Drawing.Color]::Black)
            $targetDc = $graphics.GetHdc()
            $captured = [OpenDeskTools.VisualQa.NativeWindow]::PrintWindow(
                $Window,
                $targetDc,
                [OpenDeskTools.VisualQa.NativeWindow]::PrintWindowEntireWindow
            )
        }
        finally {
            if ($targetDc -ne [IntPtr]::Zero -and $null -ne $graphics) {
                $graphics.ReleaseHdc($targetDc)
                $targetDc = [IntPtr]::Zero
            }
            if ($null -ne $graphics) {
                $graphics.Dispose()
                $graphics = $null
            }
        }
        [void](Assert-TargetWindowIdentity -ProcessId $ProcessId -Window $Window)
        if (-not $captured -or -not (Test-ImageHasVisibleDetail -Bitmap $bitmap)) {
            $bitmap.Dispose()
            $bitmap = $null
            return $null
        }
        $result = $bitmap
        $bitmap = $null
        return $result
    }
    catch {
        if ($null -ne $bitmap) {
            $bitmap.Dispose()
        }
        throw
    }
}

function Invoke-ScreenCapture {
    param(
        [IntPtr]$Window,
        [pscustomobject]$Bounds,
        [int]$ProcessId
    )

    [void](Assert-TargetWindowIdentity -ProcessId $ProcessId -Window $Window)
    [void][OpenDeskTools.VisualQa.NativeWindow]::ShowWindow(
        $Window,
        [OpenDeskTools.VisualQa.NativeWindow]::ShowRestore
    )
    [void][OpenDeskTools.VisualQa.NativeWindow]::SetForegroundWindow($Window)
    Start-Sleep -Milliseconds 250
    if ([OpenDeskTools.VisualQa.NativeWindow]::GetForegroundWindow() -ne $Window) {
        throw "Target window could not be confirmed as the foreground window."
    }
    [void](Assert-TargetWindowIdentity -ProcessId $ProcessId -Window $Window)

    $bitmap = $null
    $graphics = $null
    try {
        try {
            $bitmap = [Drawing.Bitmap]::new(
                $Bounds.Width,
                $Bounds.Height,
                [Drawing.Imaging.PixelFormat]::Format32bppArgb
            )
            $graphics = [Drawing.Graphics]::FromImage($bitmap)
            $graphics.CopyFromScreen(
                $Bounds.Left,
                $Bounds.Top,
                0,
                0,
                [Drawing.Size]::new($Bounds.Width, $Bounds.Height),
                [Drawing.CopyPixelOperation]::SourceCopy
            )
        }
        finally {
            if ($null -ne $graphics) {
                $graphics.Dispose()
                $graphics = $null
            }
        }
        [void](Assert-TargetWindowIdentity -ProcessId $ProcessId -Window $Window)
        $result = $bitmap
        $bitmap = $null
        return $result
    }
    catch {
        if ($null -ne $bitmap) {
            $bitmap.Dispose()
        }
        throw
    }
}

$targetWindow = Get-TargetWindow -ProcessId $TargetProcessId
[void][OpenDeskTools.VisualQa.NativeWindow]::ShowWindow(
    $targetWindow.Handle,
    [OpenDeskTools.VisualQa.NativeWindow]::ShowRestore
)
Start-Sleep -Milliseconds 100
$captureBounds = Get-CaptureRect -Window $targetWindow.Handle
$captureMethod = $Mode
$capturedBitmap = $null

if ($Mode -in @("Auto", "PrintWindow")) {
    $printWindowArguments = @{
        Window = $targetWindow.Handle
        Bounds = $captureBounds
        ProcessId = $targetWindow.ProcessId
    }
    $capturedBitmap = Invoke-PrintWindowCapture @printWindowArguments
    if ($null -ne $capturedBitmap) {
        $captureMethod = "PrintWindow"
    }
    elseif ($Mode -eq "PrintWindow") {
        throw "PrintWindow did not return a usable image."
    }
}

if ($null -eq $capturedBitmap) {
    $screenCaptureArguments = @{
        Window = $targetWindow.Handle
        Bounds = $captureBounds
        ProcessId = $targetWindow.ProcessId
    }
    $capturedBitmap = Invoke-ScreenCapture @screenCaptureArguments
    $captureMethod = "Screen"
}

$identityArguments = @{
    ProcessId = $targetWindow.ProcessId
    Window = $targetWindow.Handle
}
[void](Assert-TargetWindowIdentity @identityArguments)
[IO.Directory]::CreateDirectory($outputDirectory) | Out-Null

$fileMode = if ($Force) {
    [IO.FileMode]::Create
}
else {
    [IO.FileMode]::CreateNew
}
$outputStream = $null
try {
    $outputStream = [IO.File]::Open(
        $resolvedOutputPath,
        $fileMode,
        [IO.FileAccess]::Write,
        [IO.FileShare]::None
    )
    $capturedBitmap.Save($outputStream, [Drawing.Imaging.ImageFormat]::Png)
}
finally {
    if ($null -ne $outputStream) {
        $outputStream.Dispose()
    }
    $capturedBitmap.Dispose()
}

[pscustomobject]@{
    processId = $TargetProcessId
    executable = $targetWindow.Executable
    windowTitle = $targetWindow.Title
    method = $captureMethod
    left = $captureBounds.Left
    top = $captureBounds.Top
    width = $captureBounds.Width
    height = $captureBounds.Height
    outputPath = $resolvedOutputPath
} | ConvertTo-Json -Depth 3
