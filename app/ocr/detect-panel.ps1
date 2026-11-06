# Finds the exact bounds of the addon's black panel around a known region (the
# calibrated OCR area, which lies inside the panel). Grows each edge outwards while
# the next column/row still looks like the panel: mostly pure black and only
# grayscale (the panel is black with white text; game scenery is rarely exact gray).
# Prints {"x":..,"y":..,"w":..,"h":..} in physical pixels, or {"error":"..."}.
param(
  [Parameter(Mandatory = $true)][int]$X,
  [Parameter(Mandatory = $true)][int]$Y,
  [Parameter(Mandatory = $true)][int]$W,
  [Parameter(Mandatory = $true)][int]$H
)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms

Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @"
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

public static class WctPanel {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();

  static byte[] px; static int stride, width, height;

  static bool Panelish(int x0, int y0, int x1, int y1) {
    // Inclusive-exclusive line of pixels; true when it looks like panel background.
    int total = 0, black = 0, gray = 0;
    for (int y = y0; y < y1; y++) for (int x = x0; x < x1; x++) {
      int i = y * stride + x * 4;
      int b = px[i], g = px[i + 1], r = px[i + 2];
      total++;
      if (r <= 3 && g <= 3 && b <= 3) black++;
      if (Math.Abs(r - g) <= 3 && Math.Abs(g - b) <= 3) gray++;
    }
    return total > 0 && black >= total * 0.5 && gray >= total * 0.98;
  }

  public static int[] Detect(int ox, int oy, int sw, int sh, int rx, int ry, int rw, int rh) {
    using (var bmp = new Bitmap(sw, sh, PixelFormat.Format32bppArgb)) {
      using (var g = Graphics.FromImage(bmp)) g.CopyFromScreen(ox, oy, 0, 0, new Size(sw, sh));
      var data = bmp.LockBits(new Rectangle(0, 0, sw, sh), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
      stride = data.Stride; width = sw; height = sh;
      px = new byte[stride * sh];
      Marshal.Copy(data.Scan0, px, 0, px.Length);
      bmp.UnlockBits(data);
    }
    int left = Math.Max(0, rx - ox), top = Math.Max(0, ry - oy);
    int right = Math.Min(width, left + rw), bottom = Math.Min(height, top + rh);
    bool grew = true;
    while (grew) {
      grew = false;
      if (left > 0 && Panelish(left - 1, top, left, bottom)) { left--; grew = true; }
      if (right < width && Panelish(right, top, right + 1, bottom)) { right++; grew = true; }
      if (top > 0 && Panelish(left, top - 1, right, top)) { top--; grew = true; }
      if (bottom < height && Panelish(left, bottom, right, bottom + 1)) { bottom++; grew = true; }
    }
    return new int[] { ox + left, oy + top, right - left, bottom - top };
  }
}
"@
[WctPanel]::SetProcessDPIAware() | Out-Null
try {
  $vs = [System.Windows.Forms.SystemInformation]::VirtualScreen
  $r = [WctPanel]::Detect($vs.X, $vs.Y, $vs.Width, $vs.Height, $X, $Y, $W, $H)
  [Console]::Out.WriteLine((@{ x = $r[0]; y = $r[1]; w = $r[2]; h = $r[3] } | ConvertTo-Json -Compress))
} catch {
  [Console]::Out.WriteLine((@{ error = $_.Exception.Message } | ConvertTo-Json -Compress))
}
