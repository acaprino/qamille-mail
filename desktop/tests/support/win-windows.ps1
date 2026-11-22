# Window facts the native tests cannot get from WebView2's DevTools: a process's top-level windows
# (class, title, owner, enabled), a window's system menu, and messages posted to them.
#   windows <pid>             JSON list of the visible top-level windows of the process (class, title,
#                             enabled, owner, minimized, style)
#   menu <hwnd>               JSON list of the system menu entries of the window
#   post <hwnd> <msg> <wparam> posts the message (WM_SYSCOMMAND to a window, WM_COMMAND to a dialog)
#   foreground 0              the window in the foreground, as a number
param([Parameter(Mandatory)] [string]$Action, [Parameter(Mandatory)] [long]$Target, [long]$Message = 0, [long]$WParam = 0)
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class QamilleWindows {
  delegate bool EnumProc(IntPtr hwnd, IntPtr lparam);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback, IntPtr lparam);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassNameW(IntPtr hwnd, StringBuilder text, int size);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowTextW(IntPtr hwnd, StringBuilder text, int size);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hwnd);
  [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr hwnd);
  [DllImport("user32.dll")] static extern bool IsIconic(IntPtr hwnd);
  [DllImport("user32.dll")] static extern int GetWindowLongW(IntPtr hwnd, int index);
  [DllImport("user32.dll")] static extern IntPtr GetWindow(IntPtr hwnd, uint command);
  [DllImport("user32.dll")] static extern IntPtr GetSystemMenu(IntPtr hwnd, bool revert);
  [DllImport("user32.dll")] static extern int GetMenuItemCount(IntPtr menu);
  [DllImport("user32.dll")] static extern uint GetMenuItemID(IntPtr menu, int position);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetMenuStringW(IntPtr menu, uint item, StringBuilder text, int size, uint flags);
  [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr hwnd, uint message, IntPtr wparam, IntPtr lparam);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  static string Text(IntPtr hwnd, bool cls) {
    var text = new StringBuilder(256);
    if (cls) GetClassNameW(hwnd, text, 256); else GetWindowTextW(hwnd, text, 256);
    return text.ToString();
  }
  public static List<Dictionary<string, object>> Windows(uint pid) {
    var found = new List<Dictionary<string, object>>();
    EnumWindows((hwnd, _) => {
      uint owner;
      GetWindowThreadProcessId(hwnd, out owner);
      if (owner == pid && IsWindowVisible(hwnd)) found.Add(new Dictionary<string, object> {
        { "hwnd", hwnd.ToInt64() }, { "cls", Text(hwnd, true) }, { "title", Text(hwnd, false) },
        { "enabled", IsWindowEnabled(hwnd) }, { "owner", GetWindow(hwnd, 4).ToInt64() },
        { "minimized", IsIconic(hwnd) }, { "style", (uint)GetWindowLongW(hwnd, -16) } });
      return true;
    }, IntPtr.Zero);
    return found;
  }
  public static List<Dictionary<string, object>> Menu(IntPtr hwnd) {
    var entries = new List<Dictionary<string, object>>();
    var menu = GetSystemMenu(hwnd, false);
    for (int i = 0; i < GetMenuItemCount(menu); i++) {
      var text = new StringBuilder(256);
      GetMenuStringW(menu, (uint)i, text, 256, 0x400);
      entries.Add(new Dictionary<string, object> { { "id", GetMenuItemID(menu, i) }, { "text", text.ToString() } });
    }
    return entries;
  }
}
'@
switch ($Action) {
  'windows' { ConvertTo-Json -Compress -InputObject @([QamilleWindows]::Windows([uint32]$Target)) }
  'menu' { ConvertTo-Json -Compress -InputObject @([QamilleWindows]::Menu([IntPtr]$Target)) }
  'post' { [void][QamilleWindows]::PostMessageW([IntPtr]$Target, [uint32]$Message, [IntPtr]$WParam, [IntPtr]::Zero); 'posted' }
  'foreground' { [QamilleWindows]::GetForegroundWindow().ToInt64() }
  default { throw "unknown action $Action" }
}
