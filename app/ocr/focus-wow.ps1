# Brings the World of Warcraft window to the front (restoring it if minimized).
# Only changes which window is active: it never sends keystrokes. Windows allows it
# because this script is started by the app while the app has the focus (the user
# just pressed Enter in the compose box). Prints {"ok":true} or {"ok":false,"error":...}.
$ErrorActionPreference = "Stop"
Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
public static class FocusWow {
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr FindWindow(string cls, string title);
  [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
  public static string Run() {
    IntPtr game = FindWindow(null, "World of Warcraft");
    if (game == IntPtr.Zero) return "{\"ok\":false,\"error\":\"WoW window not found\"}";
    if (IsIconic(game)) ShowWindow(game, 9); // SW_RESTORE
    SetForegroundWindow(game);
    return GetForegroundWindow() == game ? "{\"ok\":true}" : "{\"ok\":false,\"error\":\"Windows refused to change the foreground window\"}";
  }
}
'@
[Console]::Out.WriteLine([FocusWow]::Run())
