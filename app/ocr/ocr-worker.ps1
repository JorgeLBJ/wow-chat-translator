# Long-running OCR worker.
# Captures a screen region every IntervalMs, and only when the pixels changed runs
# Windows OCR and writes one JSON line to stdout: {"lines":["...", ...]} or {"error":"..."}.
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File ocr-worker.ps1 -X 0 -Y 0 -W 500 -H 200 [-Scale 2] [-IntervalMs 700] [-Lang ""]
param(
  [Parameter(Mandatory = $true)][int]$X,
  [Parameter(Mandatory = $true)][int]$Y,
  [Parameter(Mandatory = $true)][int]$W,
  [Parameter(Mandatory = $true)][int]$H,
  [double]$Scale = 2,
  [int]$IntervalMs = 700,
  [string]$Lang = ""
)
$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Runtime.WindowsRuntime

Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @"
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

public static class WctCapture {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();

  public static Bitmap Grab(int x, int y, int w, int h) {
    var bmp = new Bitmap(w, h, PixelFormat.Format32bppArgb);
    using (var g = Graphics.FromImage(bmp)) { g.CopyFromScreen(x, y, 0, 0, new Size(w, h)); }
    return bmp;
  }

  public static ulong Hash(Bitmap bmp) {
    var rect = new Rectangle(0, 0, bmp.Width, bmp.Height);
    var data = bmp.LockBits(rect, ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
    int n = Math.Abs(data.Stride) * bmp.Height;
    var buf = new byte[n];
    Marshal.Copy(data.Scan0, buf, 0, n);
    bmp.UnlockBits(data);
    ulong h = 14695981039346656037UL;
    for (int i = 0; i < n; i += 4) {
      h ^= buf[i]; h *= 1099511628211UL;
      h ^= buf[i + 1]; h *= 1099511628211UL;
      h ^= buf[i + 2]; h *= 1099511628211UL;
    }
    return h;
  }
}
"@
# Region coordinates are physical pixels; without this, Windows scaling would shift the capture.
[WctCapture]::SetProcessDPIAware() | Out-Null

$asTaskGeneric = ([System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {
    $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and
    $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1' })[0]
function Await($op, [Type]$resultType) {
  $task = $asTaskGeneric.MakeGenericMethod($resultType).Invoke($null, @($op))
  $task.Wait() | Out-Null
  $task.Result
}

[Windows.Media.Ocr.OcrEngine, Windows.Foundation, ContentType = WindowsRuntime] | Out-Null
[Windows.Graphics.Imaging.BitmapDecoder, Windows.Foundation, ContentType = WindowsRuntime] | Out-Null
[Windows.Storage.Streams.InMemoryRandomAccessStream, Windows.Foundation, ContentType = WindowsRuntime] | Out-Null

if ($Lang) {
  $engine = [Windows.Media.Ocr.OcrEngine]::TryCreateFromLanguage((New-Object Windows.Globalization.Language $Lang))
} else {
  $engine = [Windows.Media.Ocr.OcrEngine]::TryCreateFromUserProfileLanguages()
}

function Emit($obj) {
  [Console]::Out.WriteLine(($obj | ConvertTo-Json -Compress))
  [Console]::Out.Flush()
}

function Recognize([System.Drawing.Bitmap]$bmp) {
  $w = [int]($bmp.Width * $Scale); $h = [int]($bmp.Height * $Scale)
  $big = New-Object System.Drawing.Bitmap $w, $h
  $g = [System.Drawing.Graphics]::FromImage($big)
  $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
  $g.DrawImage($bmp, 0, 0, $w, $h)
  $g.Dispose()
  $ms = New-Object System.IO.MemoryStream
  $big.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
  $big.Dispose()

  $stream = New-Object Windows.Storage.Streams.InMemoryRandomAccessStream
  $writer = New-Object Windows.Storage.Streams.DataWriter $stream
  $writer.WriteBytes($ms.ToArray())
  Await ($writer.StoreAsync()) ([UInt32]) | Out-Null
  $stream.Seek(0)
  $decoder = Await ([Windows.Graphics.Imaging.BitmapDecoder]::CreateAsync($stream)) ([Windows.Graphics.Imaging.BitmapDecoder])
  $soft = Await ($decoder.GetSoftwareBitmapAsync()) ([Windows.Graphics.Imaging.SoftwareBitmap])
  $result = Await ($engine.RecognizeAsync($soft)) ([Windows.Media.Ocr.OcrResult])
  $stream.Dispose()
  , @($result.Lines | ForEach-Object { $_.Text })
}

$lastHash = [UInt64]0
while ($true) {
  try {
    $bmp = [WctCapture]::Grab($X, $Y, $W, $H)
    $hash = [WctCapture]::Hash($bmp)
    if ($hash -ne $lastHash) {
      $lastHash = $hash
      $lines = Recognize $bmp
      Emit @{ lines = $lines }
    }
    $bmp.Dispose()
  } catch {
    Emit @{ error = $_.Exception.Message }
  }
  Start-Sleep -Milliseconds $IntervalMs
}
