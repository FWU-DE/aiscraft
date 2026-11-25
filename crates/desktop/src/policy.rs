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
const IDE: &str = "Programmierumgebungen und Editoren mit Terminal oder Makros";
const REMOTE: &str = "Fernwartung und Fernzugriff";
const DIALOG: &str = "Dateidialoge (Öffnen, Speichern unter)";

/// Browsers: their address bar reaches any page and `javascript:`; steps there always ask, no
/// developer tools, no address other than http(s), never the browser's password pages.
const BROWSERS: &[&str] = &[
    "msedge.exe",
    "chrome.exe",
    "firefox.exe",
    "brave.exe",
    "opera.exe",
    "opera_gx.exe",
    "vivaldi.exe",
    "arc.exe",
    "iexplore.exe",
    "waterfox.exe",
    "librewolf.exe",
    "chromium.exe",
];

pub fn is_browser(exe: &str) -> bool {
    BROWSERS.contains(&exe)
}

/// Window classes that are refused: the Office VBA editor runs code.
const BLOCKED_CLASSES: &[(&str, &str)] = &[("wndclass_desked_gsk", IDE)];

/// Classes of elements inside a window that mark a file dialog or a folder view (a „Speichern
/// unter“ can put a file into Autostart, a folder view can start programs).
const DIALOG_PARTS: &[&str] = &["shelldll_defview", "duiviewwndclassname", "address band root"];

/// Words in a browser window's title that mark its password pages.
const PASSWORD_PAGES: &[&str] =
    &["passwörter", "kennwörter", "passwords", "password manager", "passwortmanager", "about:logins", "edge://wallet", "chrome://password-manager", "wallet"];

/// Text that would open something other than a web page when typed into a browser's address bar.
pub fn browser_text_refusal(text: &str) -> Option<String> {
    let t = text.trim_start().to_lowercase();
    let schemes = [
        "javascript:",
        "file:",
        "data:",
        "vbscript:",
        "about:",
        "chrome:",
        "edge:",
        "brave:",
        "opera:",
        "vivaldi:",
        "view-source:",
        "ms-",
        "search-ms:",
        "shell:",
        "res:",
    ];
    schemes
        .iter()
        .find(|s| t.starts_with(*s))
        .map(|s| format!("In einem Browser schreibt AIS.ChatCraft nichts, das mit „{s}“ beginnt: nur gewöhnliche Text und http(s)-Adressen."))
}

/// Why a read window is refused because of what it is, not which program owns it: a file dialog
/// or folder view, the VBA editor, a browser's password page.
pub fn snapshot_refusal(s: &crate::Snapshot) -> Option<String> {
    let class = s.window.class.to_lowercase();
    if let Some((_, why)) = BLOCKED_CLASSES.iter().find(|(c, _)| *c == class) {
        return Some(format!("Dieses Fenster ist gesperrt: {why} bedient AIS.ChatCraft nie."));
    }
    if s.controls.iter().any(|c| DIALOG_PARTS.contains(&c.class.to_lowercase().as_str())) {
        return Some(format!("Dieses Fenster ist gesperrt: {DIALOG} bedient AIS.ChatCraft nie. Bitte selbst speichern oder öffnen."));
    }
    browser_window_refusal(&s.window)
}

fn browser_window_refusal(w: &WindowInfo) -> Option<String> {
    let browser = w.programs.iter().any(|p| is_browser(&p.exe));
    let title = w.title.to_lowercase();
    (browser && PASSWORD_PAGES.iter().any(|p| title.contains(p))).then(|| "Die Passwort-Seiten des Browsers bedient AIS.ChatCraft nie.".to_string())
}

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
    ("py.exe", TERMINAL),
    ("pyw.exe", TERMINAL),
    ("node.exe", TERMINAL),
    ("java.exe", TERMINAL),
    ("javaw.exe", TERMINAL),
    ("perl.exe", TERMINAL),
    ("ruby.exe", TERMINAL),
    ("rubyw.exe", TERMINAL),
    ("conemu.exe", TERMINAL),
    ("conemu64.exe", TERMINAL),
    ("tabby.exe", TERMINAL),
    ("hyper.exe", TERMINAL),
    ("cmder.exe", TERMINAL),
    ("fluent terminal.exe", TERMINAL),
    // Editors and IDEs that run code or have a terminal inside.
    ("code.exe", IDE),
    ("code - insiders.exe", IDE),
    ("cursor.exe", IDE),
    ("windsurf.exe", IDE),
    ("zed.exe", IDE),
    ("devenv.exe", IDE),
    ("idea64.exe", IDE),
    ("idea.exe", IDE),
    ("pycharm64.exe", IDE),
    ("webstorm64.exe", IDE),
    ("rider64.exe", IDE),
    ("clion64.exe", IDE),
    ("goland64.exe", IDE),
    ("phpstorm64.exe", IDE),
    ("rubymine64.exe", IDE),
    ("datagrip64.exe", IDE),
    ("studio64.exe", IDE),
    ("eclipse.exe", IDE),
    ("notepad++.exe", IDE),
    ("sublime_text.exe", IDE),
    ("atom.exe", IDE),
    ("thonny.exe", IDE),
    ("spyder.exe", IDE),
    // Remote access: someone else's computer, or this one for someone else.
    ("teamviewer.exe", REMOTE),
    ("teamviewer_desktop.exe", REMOTE),
    ("anydesk.exe", REMOTE),
    ("rustdesk.exe", REMOTE),
    ("quickassist.exe", REMOTE),
    ("msra.exe", REMOTE),
    ("mstsc.exe", REMOTE),
    ("msrdc.exe", REMOTE),
    ("vmconnect.exe", REMOTE),
    ("parsecd.exe", REMOTE),
    ("splashtop.exe", REMOTE),
    ("vncviewer.exe", REMOTE),
    ("tvnviewer.exe", REMOTE),
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
    ("\\microsoft vs code", IDE),
    ("\\jetbrains\\", IDE),
    ("\\notepad++", IDE),
    ("\\teamviewer", REMOTE),
    ("\\anydesk", REMOTE),
    ("\\rustdesk", REMOTE),
    ("\\windowsapps\\microsoftcorporationii.quickassist", REMOTE),
    ("\\python", TERMINAL),
    ("\\java\\", TERMINAL),
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
    let class = w.class.to_lowercase();
    if let Some((_, why)) = BLOCKED_CLASSES.iter().find(|(c, _)| *c == class) {
        return Some(format!("Dieses Fenster ist gesperrt: {why} bedient AIS.ChatCraft nie."));
    }
    browser_window_refusal(w)
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
            // Interpreters, IDEs, editors with macros or terminals, more terminals.
            r"C:\Program Files\Java\jdk-21\bin\javaw.exe",
            r"C:\Windows\py.exe",
            r"C:\Users\a\AppData\Local\Programs\Microsoft VS Code\Code.exe",
            r"C:\Program Files\JetBrains\IntelliJ IDEA 2026.1\bin\idea64.exe",
            r"C:\Program Files\Notepad++\notepad++.exe",
            r"C:\Program Files\ConEmu\ConEmu64.exe",
            // Remote access.
            r"C:\Program Files\TeamViewer\TeamViewer.exe",
            r"C:\Program Files (x86)\AnyDesk\AnyDesk.exe",
            r"C:\Program Files\RustDesk\rustdesk.exe",
            r"C:\Program Files\WindowsApps\MicrosoftCorporationII.QuickAssist_2.0_x64__8wekyb3d8bbwe\QuickAssist.exe",
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
        let w = |programs: Vec<Program>| WindowInfo { id: "w1".into(), title: "Fenster".into(), pid: 10, programs, class: String::new() };
        assert!(window_refusal(&w(vec![host.clone(), calc.clone()]), 1, WIN, &[]).is_none(), "a store app in its host");
        assert!(window_refusal(&w(vec![host.clone(), settings]), 1, WIN, &[]).is_some(), "the settings app in the same host");
        assert!(window_refusal(&w(vec![host]), 1, WIN, &[]).is_some(), "a host alone is not recognised");
        assert!(window_refusal(&w(vec![calc.clone()]), 10, WIN, &[]).unwrap().contains("AIS.ChatCraft"), "our own process");
        let word = Program::new(r"C:\Program Files\Microsoft Office\root\Office16\WINWORD.EXE");
        for entry in ["Word", "winword", "WINWORD.EXE", r"C:\Program Files\Microsoft Office"] {
            assert!(window_refusal(&w(vec![word.clone()]), 1, WIN, &[entry.to_string()]).unwrap().contains("Hier nie"), "{entry}");
        }
        assert!(window_refusal(&w(vec![word]), 1, WIN, &["Excel".into(), String::new()]).is_none());
        // The VBA editor (inside Word's process) and a browser's password page.
        let vbe = WindowInfo { class: "wndclass_desked_gsk".into(), ..w(vec![Program::new(r"C:\Program Files\Microsoft Office\root\Office16\WINWORD.EXE")]) };
        assert!(window_refusal(&vbe, 1, WIN, &[]).is_some());
        let edge = Program::new(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe");
        let pw = WindowInfo { title: "Passwörter – Microsoft Edge".into(), ..w(vec![edge.clone()]) };
        assert!(window_refusal(&pw, 1, WIN, &[]).is_some());
        assert!(window_refusal(&WindowInfo { title: "Elternbrief – Microsoft Edge".into(), ..w(vec![edge]) }, 1, WIN, &[]).is_none());
        // A file dialog inside an allowed program is recognised by its folder view.
        let mut dialog = crate::Snapshot { window: w(vec![Program::new(r"C:\Windows\System32\notepad.exe")]), controls: vec![], truncated: false };
        assert!(snapshot_refusal(&dialog).is_none());
        dialog.controls.push(crate::Control { class: "SHELLDLL_DefView".into(), ..crate::fake::control("1", "Liste", "Ordneransicht", None, &[]) });
        assert!(snapshot_refusal(&dialog).unwrap().contains("Dateidialoge"));
        for bad in ["javascript:alert(1)", "  FILE:///C:/", "data:text/html,x", "edge://settings", "view-source:https://x"] {
            assert!(browser_text_refusal(bad).is_some(), "{bad}");
        }
        assert!(browser_text_refusal("https://schule.example").is_none() && browser_text_refusal("Elternabend 7b").is_none());
        assert_eq!(known_app("Taschenrechner").map(|a| a.exe), Some("calc.exe"));
        assert_eq!(known_app("WORD").map(|a| a.name), Some("Word"));
        assert!(known_app("cmd").is_none());
    }
}
