param([Parameter(Mandatory = $true)][string]$ReportPath)

$ErrorActionPreference = 'Stop'
$report = (Resolve-Path -LiteralPath $ReportPath).Path
Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;

public static class CiReportBrowser {
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct ShellExecuteInfo {
        public int cbSize;
        public uint fMask;
        public IntPtr hwnd;
        public string lpVerb;
        public string lpFile;
        public string lpParameters;
        public string lpDirectory;
        public int nShow;
        public IntPtr hInstApp;
        public IntPtr lpIDList;
        public string lpClass;
        public IntPtr hkeyClass;
        public uint dwHotKey;
        public IntPtr hIcon;
        public IntPtr hProcess;
    }

    [DllImport("shell32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool ShellExecuteEx(ref ShellExecuteInfo info);

    public static void Open(string path) {
        var info = new ShellExecuteInfo {
            cbSize = Marshal.SizeOf(typeof(ShellExecuteInfo)),
            // Finish shell handoff before this helper exits; log errors without UI.
            fMask = 0x00000100 | 0x00000400,
            lpVerb = "open",
            lpFile = path,
            nShow = 4 // SW_SHOWNOACTIVATE: request background display.
        };
        if (!ShellExecuteEx(ref info))
            throw new Win32Exception(Marshal.GetLastWin32Error());
    }
}
'@
[CiReportBrowser]::Open($report)
Write-Output "Requested background opening: $report"
