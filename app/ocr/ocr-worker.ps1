# Long-running OCR worker.
# Captures a screen region every IntervalMs and writes one JSON line per capture:
#   {"panel":false}                      region does not look like the addon panel (no OCR run)
#   {"panel":true}                       panel visible, pixels unchanged (no OCR run)
#   {"panel":true,"lines":["...", ...]}  panel visible and changed: OCR result
#   {"error":"..."}
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
    using (var g = Graphics.FromImage(bmp)) {
      g.CopyFromScreen(x, y, 0, 0, new Size(w, h));
      GameCapture.PaintOver(g, x, y);
    }
    return bmp;
  }

  // The addon panel is pure black with white/gray text; game scenery almost never is.
  public static bool LooksLikePanel(Bitmap bmp) {
    var rect = new Rectangle(0, 0, bmp.Width, bmp.Height);
    var data = bmp.LockBits(rect, ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
    int n = Math.Abs(data.Stride) * bmp.Height;
    var buf = new byte[n];
    Marshal.Copy(data.Scan0, buf, 0, n);
    bmp.UnlockBits(data);
    long total = 0, black = 0, gray = 0;
    for (int i = 0; i < n; i += 8) {
      int b = buf[i], g = buf[i + 1], r = buf[i + 2];
      total++;
      if (r <= 3 && g <= 3 && b <= 3) black++;
      if (Math.Abs(r - g) <= 3 && Math.Abs(g - b) <= 3) gray++;
    }
    return total > 0 && black >= total * 0.5 && gray >= total * 0.98;
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
// Captures the World of Warcraft window itself (PrintWindow + PW_RENDERFULLCONTENT)
// and paints it over a screen capture. Windows draws capture-protected windows (our
// overlay) as black in screen captures over a DirectX game, so reading the screen
// under the overlay is unreliable; the game window's own image is not affected.
public static class GameCapture {
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr FindWindow(string cls, string title);
  [DllImport("user32.dll")] static extern bool IsWindow(IntPtr h);
  [DllImport("user32.dll")] static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
  [StructLayout(LayoutKind.Sequential)] struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
  static IntPtr game = IntPtr.Zero;

  // Returns true when the game window was painted over g (whose origin is screen ox, oy).
  public static bool PaintOver(Graphics g, int ox, int oy) {
    if (game == IntPtr.Zero || !IsWindow(game)) game = FindWindow(null, "World of Warcraft");
    if (game == IntPtr.Zero || IsIconic(game)) return false;
    RECT r;
    if (!GetWindowRect(game, out r)) return false;
    int w = r.R - r.L, h = r.B - r.T;
    if (w <= 0 || h <= 0) return false;
    using (var win = new Bitmap(w, h, PixelFormat.Format32bppArgb)) {
      bool ok;
      using (var wg = Graphics.FromImage(win)) {
        IntPtr dc = wg.GetHdc();
        ok = PrintWindow(game, dc, 2);
        wg.ReleaseHdc(dc);
      }
      if (!ok) return false;
      g.DrawImageUnscaled(win, r.L - ox, r.T - oy);
    }
    return true;
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
    if (-not [WctCapture]::LooksLikePanel($bmp)) {
      $lastHash = [UInt64]0 # OCR again as soon as the panel comes back
      Emit @{ panel = $false }
    } else {
      $hash = [WctCapture]::Hash($bmp)
      if ($hash -ne $lastHash) {
        $lastHash = $hash
        $lines = Recognize $bmp
        Emit @{ panel = $true; lines = $lines }
      } else {
        Emit @{ panel = $true }
      }
    }
    $bmp.Dispose()
  } catch {
    Emit @{ error = $_.Exception.Message }
  }
  Start-Sleep -Milliseconds $IntervalMs
}
