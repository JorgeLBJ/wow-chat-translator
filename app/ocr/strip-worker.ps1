# Pixel-strip capture worker.
# Grabs a small patch at the top-left of the World of Warcraft client area every
# IntervalMs and writes one JSON line to stdout:
#   {"patch":"<base64 RGB>","w":W,"h":H}  the strip is there and the patch changed
#   {"strip":false}                       no strip visible (sent when that state starts)
#   {"wow":false}                         WoW window not found (sent when that state starts)
#   {"error":"..."}
# The app decodes the patch (app/src/core/strip-reader.js).
param(
  [int]$IntervalMs = 35,
  [int]$PatchW = 260,
  [int]$PatchH = 28
)
$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false
Add-Type -AssemblyName System.Drawing

Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @"
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

public static class StripCapture {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr FindWindow(string cls, string title);
  [DllImport("user32.dll")] static extern bool IsWindow(IntPtr h);
  [DllImport("user32.dll")] static extern bool IsIconic(IntPtr h);
  [StructLayout(LayoutKind.Sequential)] struct POINT { public int X, Y; }
  [DllImport("user32.dll")] static extern bool ClientToScreen(IntPtr h, ref POINT p);

  static IntPtr game = IntPtr.Zero;
  static ulong lastHash = 0;

  // Returns null when WoW is not found, an empty array when no strip is visible,
  // or the RGB bytes of the patch when the strip is there and the image changed.
  // "unchanged" is signalled with a 1-byte array.
  public static byte[] Grab(int w, int h) {
    if (game == IntPtr.Zero || !IsWindow(game)) game = FindWindow(null, "World of Warcraft");
    if (game == IntPtr.Zero || IsIconic(game)) return null;
    var origin = new POINT();
    if (!ClientToScreen(game, ref origin)) return null;

    byte[] bgra;
    int stride;
    using (var bmp = new Bitmap(w, h, PixelFormat.Format32bppArgb)) {
      using (var g = Graphics.FromImage(bmp)) g.CopyFromScreen(origin.X, origin.Y, 0, 0, new Size(w, h));
      var data = bmp.LockBits(new Rectangle(0, 0, w, h), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
      stride = data.Stride;
      bgra = new byte[stride * h];
      Marshal.Copy(data.Scan0, bgra, 0, bgra.Length);
      bmp.UnlockBits(data);
    }

    if (!HasMarker(bgra, stride, w, h)) { lastHash = 0; return new byte[0]; }

    var rgb = new byte[w * h * 3];
    ulong hash = 14695981039346656037UL;
    for (int y = 0; y < h; y++) {
      for (int x = 0; x < w; x++) {
        int s = y * stride + x * 4, d = (y * w + x) * 3;
        rgb[d] = bgra[s + 2]; rgb[d + 1] = bgra[s + 1]; rgb[d + 2] = bgra[s];
        hash ^= rgb[d]; hash *= 1099511628211UL;
        hash ^= rgb[d + 1]; hash *= 1099511628211UL;
        hash ^= rgb[d + 2]; hash *= 1099511628211UL;
      }
    }
    if (hash == lastHash) return new byte[1];
    lastHash = hash;
    return rgb;
  }

  // Same quick test as the app: the first marker cell (levels 15, 0, 8) is strong red
  // with no green, for some cell size 1-4 and offset 0-2.
  static bool HasMarker(byte[] bgra, int stride, int w, int h) {
    for (int cell = 1; cell <= 4; cell++)
      for (int dy = 0; dy <= 2; dy++)
        for (int dx = 0; dx <= 2; dx++) {
          int x = dx + cell / 2, y = dy + cell / 2;
          if (x >= w || y >= h) continue;
          int o = y * stride + x * 4;
          if (bgra[o + 2] > 150 && bgra[o + 1] < 100) return true;
        }
    return false;
  }
}
"@
# Coordinates are physical pixels; without this, Windows scaling would shift the capture.
[StripCapture]::SetProcessDPIAware() | Out-Null

function Emit($obj) {
  [Console]::Out.WriteLine(($obj | ConvertTo-Json -Compress))
  [Console]::Out.Flush()
}

$state = ""
while ($true) {
  try {
    $bytes = [StripCapture]::Grab($PatchW, $PatchH)
    if ($null -eq $bytes) {
      if ($state -ne "nowow") { Emit @{ wow = $false }; $state = "nowow" }
    } elseif ($bytes.Length -eq 0) {
      if ($state -ne "nostrip") { Emit @{ strip = $false }; $state = "nostrip" }
    } elseif ($bytes.Length -gt 1) {
      Emit @{ patch = [Convert]::ToBase64String($bytes); w = $PatchW; h = $PatchH }
      $state = "strip"
    }
  } catch {
    Emit @{ error = $_.Exception.Message }
  }
  Start-Sleep -Milliseconds $IntervalMs
}
