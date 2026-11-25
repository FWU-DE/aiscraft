//! Programs computer use never touches, and the programs it knows by name.
//!
//! The hard block list is no judgement call: terminals and script hosts, system settings and
//! security, the file manager, password managers, crypto wallets and AIS.ChatCraft itself are
//! refused by program file name and path, before anything is asked. Everything inside the Windows
//! folder is refused too, except a few plain programs (Editor, Paint, Rechner); so a system tool
//! nobody listed here is refused anyway. A program whose path cannot be read (another user's,
//! a protected one) is refused. The person's own list („Hier nie“) comes on top.

use crate::{Program, WindowInfo};

/// Why a kind of program is refused, for people.
const TERMINAL: &str = "Terminals und Skript-Programme";
const SYSTEM: &str = "Systemeinstellungen und Sicherheit";
const FILES: &str = "der Dateimanager";
const PASSWORDS: &str = "Passwortmanager";
const WALLETS: &str = "Krypto-Wallets";
const SELF: &str = "AIS.ChatCraft selbst";

/// Program file names (lowercase) and why they are refused.
const BLOCKED: &[(&str, &str)] = &[
    // Terminals and script hosts.
    ("cmd.exe", TERMINAL),
    ("powershell.exe", TERMINAL),
    ("powershell_ise.exe", TERMINAL),
    ("pwsh.exe", TERMINAL),
    ("windowsterminal.exe", TERMINAL),
    ("wt.exe", TERMINAL),
    ("openconsole.exe", TERMINAL),
    ("conhost.exe", TERMINAL),
    ("wscript.exe", TERMINAL),
    ("cscript.exe", TERMINAL),
    ("mshta.exe", TERMINAL),
    ("bash.exe", TERMINAL),
    ("sh.exe", TERMINAL),
    ("wsl.exe", TERMINAL),
    ("wslhost.exe", TERMINAL),
    ("mintty.exe", TERMINAL),
    ("git-bash.exe", TERMINAL),
    ("putty.exe", TERMINAL),
    ("kitty.exe", TERMINAL),
    ("alacritty.exe", TERMINAL),
    ("wezterm-gui.exe", TERMINAL),
    ("rundll32.exe", TERMINAL),
    ("regsvr32.exe", TERMINAL),
    ("msiexec.exe", TERMINAL),
    ("python.exe", TERMINAL),
    ("pythonw.exe", TERMINAL),
    ("node.exe", TERMINAL),
    // System settings and security.
    ("systemsettings.exe", SYSTEM),
    ("control.exe", SYSTEM),
    ("regedit.exe", SYSTEM),
    ("regedt32.exe", SYSTEM),
    ("taskmgr.exe", SYSTEM),
    ("mmc.exe", SYSTEM),
    ("consent.exe", SYSTEM),
    ("credentialuibroker.exe", SYSTEM),
    ("msconfig.exe", SYSTEM),
    ("secpol.exe", SYSTEM),
    ("gpedit.exe", SYSTEM),
    ("sechealthui.exe", SYSTEM),
    ("securityhealthhost.exe", SYSTEM),
    ("securityhealthsystray.exe", SYSTEM),
    ("useraccountcontrolsettings.exe", SYSTEM),
    ("systempropertiesadvanced.exe", SYSTEM),
    ("systempropertiesprotection.exe", SYSTEM),
    ("systempropertiesremote.exe", SYSTEM),
    ("lockapp.exe", SYSTEM),
    ("logonui.exe", SYSTEM),
    ("perfmon.exe", SYSTEM),
    ("resmon.exe", SYSTEM),
    ("eventvwr.exe", SYSTEM),
    ("procexp.exe", SYSTEM),
    ("procexp64.exe", SYSTEM),
    ("autoruns.exe", SYSTEM),
    // The file manager.
    ("explorer.exe", FILES),
    // Password managers.
    ("keepass.exe", PASSWORDS),
    ("keepassxc.exe", PASSWORDS),
    ("1password.exe", PASSWORDS),
    ("bitwarden.exe", PASSWORDS),
    ("dashlane.exe", PASSWORDS),
    ("lastpass.exe", PASSWORDS),
    ("enpass.exe", PASSWORDS),
    ("roboform.exe", PASSWORDS),
    ("nordpass.exe", PASSWORDS),
    ("keeperpasswordmanager.exe", PASSWORDS),
    ("proton pass.exe", PASSWORDS),
    ("pwsafe.exe", PASSWORDS),
    ("passwordsafe.exe", PASSWORDS),
    ("sticky password.exe", PASSWORDS),
    // Crypto wallets.
    ("exodus.exe", WALLETS),
    ("electrum.exe", WALLETS),
    ("ledger live.exe", WALLETS),
    ("trezor suite.exe", WALLETS),
    ("atomic wallet.exe", WALLETS),
    ("bitcoin-qt.exe", WALLETS),
    ("monero-wallet-gui.exe", WALLETS),
    ("wasabi wallet.exe", WALLETS),
    ("sparrow.exe", WALLETS),
    ("guarda.exe", WALLETS),
    ("coinomi.exe", WALLETS),
    ("trust wallet.exe", WALLETS),
    // AIS.ChatCraft itself.
    ("aiscraft.exe", SELF),
    ("aiscraft-preview.exe", SELF),
    ("aiscraft-cli.exe", SELF),
];

/// Parts of a path (lowercase) that mark a refused program wherever it lies (store apps,
/// renamed copies).
const BLOCKED_PATHS: &[(&str, &str)] = &[
    ("\\windowsapps\\microsoft.windowsterminal", TERMINAL),
    ("\\windowsapps\\microsoft.powershell", TERMINAL),
    ("\\windowsapps\\microsoftcorporationii.windowssubsystemforlinux", TERMINAL),
    ("\\windowsapps\\microsoft.sechealthui", SYSTEM),
    ("\\windowsapps\\microsoft.windows.devhome", SYSTEM),
    ("\\keepass", PASSWORDS),
    ("\\1password", PASSWORDS),
    ("\\bitwarden", PASSWORDS),
    ("\\electrum", WALLETS),
    ("\\exodus", WALLETS),
    ("\\ledger live", WALLETS),
    ("\\ais.chatcraft", SELF),
    ("\\aischatcraft", SELF),
];

/// Plain programs in the Windows folder that may be operated.
const WINDOWS_FOLDER_OK: &[&str] = &["notepad.exe", "mspaint.exe", "calc.exe", "write.exe", "charmap.exe"];

/// Programs that only host other programs' windows (store apps): they are judged by what they host.
const HOSTS: &[&str] = &["applicationframehost.exe"];

pub fn is_host(exe: &str) -> bool {
    HOSTS.contains(&exe)
}

/// Why one program is refused, if it is. `windir` is the Windows folder (`C:\Windows`).
pub fn program_refusal(p: &Program, windir: &str) -> Option<String> {
    let path = p.path.to_lowercase().replace('/', "\\");
    if path.trim().is_empty() {
        return Some(format!("Das Programm hinter diesem Fenster ({}) ließ sich nicht erkennen (geschützt oder mit anderen Rechten gestartet).", p.exe));
    }
    let exe = path.rsplit('\\').next().unwrap_or(&path);
    if let Some((_, why)) = BLOCKED.iter().find(|(name, _)| *name == exe || *name == p.exe) {
        return Some(format!("{} ist gesperrt: {why} bedient AIS.ChatCraft nie.", p.exe));
    }
    if let Some((_, why)) = BLOCKED_PATHS.iter().find(|(part, _)| path.contains(part)) {
        return Some(format!("{} ist gesperrt: {why} bedient AIS.ChatCraft nie.", p.exe));
    }
    let windir = windir.to_lowercase().replace('/', "\\");
    let windir = windir.trim_end_matches('\\');
    if !windir.is_empty() && path.starts_with(&format!("{windir}\\")) && !WINDOWS_FOLDER_OK.contains(&exe) && !is_host(exe) {
        return Some(format!("{} ist gesperrt: Programme des Windows-Ordners ({SYSTEM}) bedient AIS.ChatCraft nie.", p.exe));
    }
    None
}

/// Whether an entry of the person's own list („Hier nie“) names this program: its file name with
/// or without `.exe`, the name of a known program („Word“), or a folder it lies in.
pub fn on_own_list(p: &Program, entry: &str) -> bool {
    let e = entry.trim().to_lowercase().replace('/', "\\");
    if e.is_empty() {
        return false;
    }
    let stem = p.exe.strip_suffix(".exe").unwrap_or(&p.exe);
    if p.exe == e || stem == e {
        return true;
    }
    if let Some(app) = known_app(&e)
        && (app.exe == p.exe || app.window_exes.contains(&p.exe.as_str()))
    {
        return true;
    }
    e.contains('\\') && p.path.to_lowercase().replace('/', "\\").starts_with(e.trim_end_matches('\\'))
}

/// Why a window is refused: any program behind it on the hard list or the person's own list, or
/// no program known at all, or AIS.ChatCraft's own process.
pub fn window_refusal(w: &WindowInfo, own_pid: u32, windir: &str, own_list: &[String]) -> Option<String> {
    if w.pid == own_pid {
        return Some(format!("Dieses Fenster gehört AIS.ChatCraft selbst; {SELF} bedient AIS.ChatCraft nie."));
    }
    let judged: Vec<&Program> = w.programs.iter().filter(|p| !is_host(&p.exe)).collect();
    if judged.is_empty() {
        return Some(format!("Das Programm hinter „{}“ ließ sich nicht erkennen.", crate::cap(&w.title, 80)));
    }
    for p in judged {
        if let Some(why) = program_refusal(p, windir) {
            return Some(why);
        }
        if let Some(entry) = own_list.iter().find(|e| on_own_list(p, e)) {
            return Some(format!("{} steht auf deiner Liste „Hier nie“ ({entry}).", p.exe));
        }
    }
    None
}

/// A program computer use can start by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownApp {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    /// The file to start.
    pub exe: &'static str,
    /// The programs its windows belong to (store apps run under another name).
    pub window_exes: &'static [&'static str],
}

pub const KNOWN_APPS: &[KnownApp] = &[
    KnownApp { name: "Editor", aliases: &["editor", "notepad", "texteditor", "notepad.exe"], exe: "notepad.exe", window_exes: &["notepad.exe"] },
    KnownApp { name: "Word", aliases: &["word", "winword", "microsoft word", "winword.exe"], exe: "winword.exe", window_exes: &["winword.exe"] },
    KnownApp { name: "Excel", aliases: &["excel", "microsoft excel", "excel.exe"], exe: "excel.exe", window_exes: &["excel.exe"] },
    KnownApp {
        name: "PowerPoint",
        aliases: &["powerpoint", "powerpnt", "microsoft powerpoint", "powerpnt.exe"],
        exe: "powerpnt.exe",
        window_exes: &["powerpnt.exe"],
    },
    KnownApp { name: "Outlook", aliases: &["outlook", "microsoft outlook", "outlook.exe"], exe: "outlook.exe", window_exes: &["outlook.exe", "olk.exe"] },
    KnownApp { name: "OneNote", aliases: &["onenote", "microsoft onenote", "onenote.exe"], exe: "onenote.exe", window_exes: &["onenote.exe"] },
    KnownApp { name: "Paint", aliases: &["paint", "mspaint", "mspaint.exe"], exe: "mspaint.exe", window_exes: &["mspaint.exe"] },
    KnownApp {
        name: "Rechner",
        aliases: &["rechner", "taschenrechner", "calculator", "calc", "calc.exe"],
        exe: "calc.exe",
        window_exes: &["calculatorapp.exe", "calc.exe", "win32calc.exe"],
    },
    KnownApp {
        name: "LibreOffice",
        aliases: &["libreoffice", "libreoffice writer", "writer", "soffice", "soffice.exe"],
        exe: "soffice.exe",
        window_exes: &["soffice.bin", "soffice.exe"],
    },
];

/// A known program by one of its names (case does not matter).
pub fn known_app(name: &str) -> Option<&'static KnownApp> {
    let n = name.trim().to_lowercase();
    KNOWN_APPS.iter().find(|a| a.aliases.contains(&n.as_str()) || a.name.to_lowercase() == n)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIN: &str = r"C:\Windows";

    fn refused(path: &str) -> bool {
        program_refusal(&Program::new(path), WIN).is_some()
    }

    #[test]
    fn the_hard_list_by_name_and_path() {
        for bad in [
            r"C:\Windows\System32\cmd.exe",
            r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
            r"C:\Program Files\PowerShell\7\pwsh.exe",
            r"C:\Program Files\WindowsApps\Microsoft.WindowsTerminal_1.21.0_x64__8wekyb3d8bbwe\WindowsTerminal.exe",
            r"C:\Windows\System32\wscript.exe",
            r"C:\Windows\System32\mshta.exe",
            r"C:\Windows\ImmersiveControlPanel\SystemSettings.exe",
            r"C:\Windows\System32\control.exe",
            r"C:\Windows\regedit.exe",
            r"C:\Windows\System32\Taskmgr.exe",
            r"C:\Windows\System32\mmc.exe",
            r"C:\Windows\System32\consent.exe",
            r"C:\Windows\explorer.exe",
            r"C:\Program Files\KeePassXC\KeePassXC.exe",
            r"C:\Users\a\AppData\Local\1Password\app\8\1Password.exe",
            r"C:\Users\a\AppData\Local\exodus\app-24.1.1\Exodus.exe",
            r"C:\Users\a\AppData\Local\Programs\AIS.ChatCraft\aiscraft.exe",
            // A system tool nobody listed: refused because it lies in the Windows folder.
            r"C:\Windows\System32\certmgr.exe",
            r"C:\WINDOWS\SYSTEM32\DiskMgmt.exe",
            // A renamed copy of a terminal somewhere else.
            r"D:\Tools\CMD.EXE",
            // No path: not recognised.
            "",
        ] {
            assert!(refused(bad), "{bad}");
        }
        for ok in [
            r"C:\Windows\System32\notepad.exe",
            r"C:\Program Files\WindowsApps\Microsoft.WindowsNotepad_11.2409.9.0_x64__8wekyb3d8bbwe\Notepad\Notepad.exe",
            r"C:\Program Files\Microsoft Office\root\Office16\WINWORD.EXE",
            r"C:\Program Files\LibreOffice\program\soffice.bin",
        ] {
            assert!(!refused(ok), "{ok}");
        }
    }

    #[test]
    fn windows_are_judged_by_what_they_show_and_the_own_list() {
        let host = Program::new(r"C:\Windows\System32\ApplicationFrameHost.exe");
        let calc = Program::new(r"C:\Program Files\WindowsApps\Microsoft.WindowsCalculator_11\CalculatorApp.exe");
        let settings = Program::new(r"C:\Windows\ImmersiveControlPanel\SystemSettings.exe");
        let w = |programs: Vec<Program>| WindowInfo { id: "w1".into(), title: "Fenster".into(), pid: 10, programs };
        assert!(window_refusal(&w(vec![host.clone(), calc.clone()]), 1, WIN, &[]).is_none(), "a store app in its host");
        assert!(window_refusal(&w(vec![host.clone(), settings]), 1, WIN, &[]).is_some(), "the settings app in the same host");
        assert!(window_refusal(&w(vec![host]), 1, WIN, &[]).is_some(), "a host alone is not recognised");
        assert!(window_refusal(&w(vec![calc.clone()]), 10, WIN, &[]).unwrap().contains("AIS.ChatCraft"), "our own process");
        let word = Program::new(r"C:\Program Files\Microsoft Office\root\Office16\WINWORD.EXE");
        for entry in ["Word", "winword", "WINWORD.EXE", r"C:\Program Files\Microsoft Office"] {
            assert!(window_refusal(&w(vec![word.clone()]), 1, WIN, &[entry.to_string()]).unwrap().contains("Hier nie"), "{entry}");
        }
        assert!(window_refusal(&w(vec![word]), 1, WIN, &["Excel".into(), String::new()]).is_none());
        assert_eq!(known_app("Taschenrechner").map(|a| a.exe), Some("calc.exe"));
        assert_eq!(known_app("WORD").map(|a| a.name), Some("Word"));
        assert!(known_app("cmd").is_none());
    }
}
