# Runs Windows OCR (Windows.Media.Ocr) on an image and prints one recognized line per output line.
# Usage: powershell -NoProfile -File win-ocr.ps1 -Path <image> [-Lang es-MX] [-Crop x,y,w,h] [-Scale 2]
param(
  [Parameter(Mandatory = $true)][string]$Path,
  [string]$Lang = "",
  [string]$Crop = "",
  [double]$Scale = 1
)
$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Runtime.WindowsRuntime

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

# Optional crop + upscale with System.Drawing, then hand PNG bytes to WinRT.
$src = [System.Drawing.Bitmap]::FromFile((Resolve-Path $Path))
$rect = New-Object System.Drawing.Rectangle 0, 0, $src.Width, $src.Height
if ($Crop) {
  $c = $Crop.Split(',') | ForEach-Object { [int]$_ }
  $rect = New-Object System.Drawing.Rectangle $c[0], $c[1], $c[2], $c[3]
}
$w = [int]($rect.Width * $Scale); $h = [int]($rect.Height * $Scale)
$dst = New-Object System.Drawing.Bitmap $w, $h
$g = [System.Drawing.Graphics]::FromImage($dst)
$g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
$g.DrawImage($src, (New-Object System.Drawing.Rectangle 0, 0, $w, $h), $rect, [System.Drawing.GraphicsUnit]::Pixel)
$g.Dispose(); $src.Dispose()
$ms = New-Object System.IO.MemoryStream
$dst.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png); $dst.Dispose()

$stream = New-Object Windows.Storage.Streams.InMemoryRandomAccessStream
$writer = New-Object Windows.Storage.Streams.DataWriter $stream
$writer.WriteBytes($ms.ToArray())
Await ($writer.StoreAsync()) ([UInt32]) | Out-Null
$stream.Seek(0)

$decoder = Await ([Windows.Graphics.Imaging.BitmapDecoder]::CreateAsync($stream)) ([Windows.Graphics.Imaging.BitmapDecoder])
$bitmap = Await ($decoder.GetSoftwareBitmapAsync()) ([Windows.Graphics.Imaging.SoftwareBitmap])

if ($Lang) {
  $engine = [Windows.Media.Ocr.OcrEngine]::TryCreateFromLanguage((New-Object Windows.Globalization.Language $Lang))
} else {
  $engine = [Windows.Media.Ocr.OcrEngine]::TryCreateFromUserProfileLanguages()
}
$result = Await ($engine.RecognizeAsync($bitmap)) ([Windows.Media.Ocr.OcrResult])
foreach ($line in $result.Lines) { $line.Text }
