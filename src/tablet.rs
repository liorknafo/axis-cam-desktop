//! Projects reach the shop tablet from the computer over Wi-Fi (LIO-459),
//! one way: the computer shares, the tablet keeps copies.
//!
//! The computer (`serve`, while the desktop app is open) answers on `PORT`:
//! over UDP a tablet looking for it (`discover`), and over TCP a tablet
//! pairing with the code the Export page shows (Pair a tablet), or a paired
//! tablet asking for the projects it shares. Pairing and its proof are the
//! Pi's (`uc100_remote::data`): a 6-digit code valid 10 minutes and voided
//! by 5 wrong tries, then an id and a secret the tablet proves with an HMAC
//! of a fresh nonce. The frames are the Pi's too (`uc100_remote::wire`).
//!
//! Shared: the open project (the workspace as last saved) and the project
//! files saved or opened lately (`Settings::recent_projects`), each sent as
//! a tablet file (`bundle`). The tablet (`sync`) fetches the new or changed
//! ones into `projects/` beside its workspace and lists them (`kept`), so
//! they stay when the computer is off. A project the computer stops sharing
//! stays on the tablet.
use crate::model::Document;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs, io,
    net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket},
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};
use uc100_remote::{
    Token,
    data::{Data, Paired, random32},
    wire,
};

/// The computer's port, TCP and UDP; the Pi's is 7110.
pub const PORT: u16 = 7111;
/// How many recent project files the computer shares besides the open one.
pub const RECENT: usize = 20;
const ASK: &[u8] = b"AXIS-SHOP?";
const ANSWER: &[u8] = b"AXIS-CAM";
/// The largest message but a tablet file.
const SMALL: usize = 1 << 20;
/// The largest tablet file.
const BUNDLE: usize = 256 << 20;
const WAIT: Duration = Duration::from_secs(10);
const NOT_PAIRED: &str =
    "This tablet is not paired with this computer, or was forgotten: pair it again.";

#[derive(Serialize, Deserialize)]
enum ToComputer {
    Pair { code: String, name: String },
    Hello { client: u32 },
    Proof([u8; 32]),
    List,
    Get(String),
}

#[derive(Serialize, Deserialize)]
enum ToTablet {
    Paired { token: String, computer: String },
    Refused(String),
    Nonce([u8; 32]),
    List(Vec<Shared>),
    Bundle(Vec<u8>),
}

/// A project the computer shares: `key` names it across changes (its file's
/// path, or "open" for an open project with no file), `version` changes
/// with it or its part.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shared {
    pub key: String,
    pub title: String,
    pub version: String,
}

// ---- The computer ----

fn tablets(state_path: &Path) -> io::Result<Data> {
    Data::open(state_path.with_file_name("tablets"))
}

/// A new pairing code for a tablet, replacing any earlier one.
pub fn new_code(state_path: &Path) -> Result<String, String> {
    tablets(state_path)
        .and_then(|d| d.new_code())
        .map_err(|e| e.to_string())
}

/// Answers tablets from background threads for as long as the app runs.
/// Another copy of the app holding the port leaves this one silent.
pub fn serve(state_path: PathBuf, computer: String) {
    match TcpListener::bind(("0.0.0.0", PORT)) {
        Ok(l) => serve_on(l, state_path, computer.clone()),
        Err(e) => eprintln!("Shop tablet sharing is off: port {PORT}: {e}"),
    }
    if let Ok(udp) = UdpSocket::bind(("0.0.0.0", PORT)) {
        std::thread::spawn(move || {
            let mut buf = [0; 64];
            while let Ok((n, from)) = udp.recv_from(&mut buf) {
                if &buf[..n] == ASK {
                    let _ = udp.send_to(&[ANSWER, b" ", computer.as_bytes()].concat(), from);
                }
            }
        });
    }
}

fn serve_on(listener: TcpListener, state_path: PathBuf, computer: String) {
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (state_path, computer) = (state_path.clone(), computer.clone());
            std::thread::spawn(move || {
                let _ = answer(stream, &state_path, &computer);
            });
        }
    });
}

/// Two tablets trying codes at once must not both count as the first.
static PAIRING: Mutex<()> = Mutex::new(());

fn answer(mut s: TcpStream, state_path: &Path, computer: &str) -> io::Result<()> {
    s.set_read_timeout(Some(WAIT))?;
    s.set_write_timeout(Some(WAIT * 6))?;
    let bad = || io::Error::new(io::ErrorKind::InvalidData, "unexpected message");
    match wire::read(&mut s, SMALL)? {
        ToComputer::Pair { code, name } => {
            let _held = PAIRING.lock().unwrap_or_else(|e| e.into_inner());
            let reply = match tablets(state_path)?.pair(&code, &name)? {
                Paired::Yes(token) => ToTablet::Paired {
                    token: token.to_string(),
                    computer: computer.into(),
                },
                Paired::No(why) => ToTablet::Refused(reword(why)),
            };
            wire::write(&mut s, &reply)
        }
        ToComputer::Hello { client } => {
            let nonce = random32();
            wire::write(&mut s, &ToTablet::Nonce(nonce))?;
            let ToComputer::Proof(mac) = wire::read(&mut s, SMALL)? else {
                return Err(bad());
            };
            let known = tablets(state_path)?.clients()?;
            if !known
                .iter()
                .any(|c| c.id == client && c.verify(&nonce, &mac))
            {
                return wire::write(&mut s, &ToTablet::Refused(NOT_PAIRED.into()));
            }
            let shared = shared(state_path);
            loop {
                let reply = match wire::read(&mut s, SMALL) {
                    Ok(ToComputer::List) => {
                        ToTablet::List(shared.iter().map(|(s, _)| s.clone()).collect())
                    }
                    Ok(ToComputer::Get(key)) => match shared.iter().find(|(s, _)| s.key == key) {
                        Some((_, doc)) => match crate::bundle::pack(doc) {
                            Ok(bytes) => ToTablet::Bundle(bytes),
                            Err(e) => ToTablet::Refused(e),
                        },
                        None => ToTablet::Refused("The computer no longer shares it.".into()),
                    },
                    Ok(_) => return Err(bad()),
                    // The tablet has what it wanted.
                    Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
                    Err(e) => return Err(e),
                };
                wire::write(&mut s, &reply)?;
            }
        }
        _ => Err(bad()),
    }
}

/// The Pi's refusals tell to run `uc100-server pair` on the Pi.
fn reword(why: String) -> String {
    why.replace(
        "run `sudo uc100-server pair` on the Pi for one",
        "on the computer's Export page, click Pair a tablet for one",
    )
    .replace(
        "run `sudo uc100-server pair` again",
        "click Pair a tablet on the computer again",
    )
}

/// What the computer shares, read from its files as the app last saved
/// them: the open project first, then the recent project files. A project
/// with no setups, or whose part is gone, is left out.
fn shared(state_path: &Path) -> Vec<(Shared, Document)> {
    let settings: crate::settings::Settings = fs::read(crate::settings::path(state_path))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let open = settings.project_file.clone();
    let recent = (settings.recent_projects.iter())
        .filter(|p| Some(*p) != open.as_ref())
        .take(RECENT)
        .map(|p| (key_of(p), p.as_path()));
    let open_key = open.as_deref().map_or("open".into(), key_of);
    std::iter::once((open_key, state_path))
        .chain(recent)
        .filter_map(|(key, path)| {
            let bytes = fs::read(path).ok()?;
            let doc = crate::storage::load(path).ok()?;
            if doc.setups.is_empty() {
                return None;
            }
            let mut h = Sha256::new();
            h.update(&bytes);
            // The files the tablet file carries besides the project: its
            // part, and its machines' profiles (`bundle::pack`).
            let part = doc.file.as_ref().map(|f| f.path.as_path());
            let profiles = crate::bundle::profiles_of(&doc).map(Path::new);
            for path in part.into_iter().chain(profiles) {
                let meta = fs::metadata(path).ok()?;
                h.update(path.to_string_lossy().as_bytes());
                h.update(meta.len().to_le_bytes());
                h.update(format!("{:?}", meta.modified().ok()).as_bytes());
            }
            let shared = Shared {
                key,
                title: doc.title.clone(),
                version: hex(&h.finalize()[..8]),
            };
            Some((shared, doc))
        })
        .collect()
}

fn key_of(path: &Path) -> String {
    hex(&Sha256::digest(path.to_string_lossy().as_bytes())[..8])
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---- The tablet ----

/// The computer this tablet is paired with, kept beside its workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Link {
    pub address: String,
    pub computer: String,
    pub token: String,
}

fn link_file(dir: &Path) -> PathBuf {
    dir.join("computer.json")
}

fn projects(dir: &Path) -> PathBuf {
    dir.join("projects")
}

pub fn link(dir: &Path) -> Option<Link> {
    serde_json::from_slice(&fs::read(link_file(dir)).ok()?).ok()
}

fn keep_link(dir: &Path, link: &Link) -> Result<(), String> {
    let json = serde_json::to_vec_pretty(link).map_err(|e| e.to_string())?;
    crate::storage::write_atomic(&link_file(dir), &json)
}

/// Forgets the computer; the projects it sent stay.
pub fn forget(dir: &Path) -> Result<(), String> {
    match fs::remove_file(link_file(dir)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}

/// The address of the computer that answers a broadcast on this Wi-Fi, if
/// one does within 1.5 s. A broadcast is sent, not received, so Android
/// needs no multicast lock for it.
pub fn discover() -> Option<String> {
    let udp = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    udp.set_broadcast(true).ok()?;
    udp.set_read_timeout(Some(Duration::from_millis(1500)))
        .ok()?;
    udp.send_to(ASK, ("255.255.255.255", PORT)).ok()?;
    let mut buf = [0; 256];
    let (n, from) = udp.recv_from(&mut buf).ok()?;
    buf[..n]
        .starts_with(ANSWER)
        .then(|| SocketAddr::new(from.ip(), PORT).to_string())
}

/// A typed address, with the port when it has none.
fn with_port(address: &str) -> String {
    let a = address.trim();
    if a.parse::<SocketAddr>().is_ok() || (a.contains(':') && !a.contains("::")) {
        a.into()
    } else {
        format!("{a}:{PORT}")
    }
}

fn connect(address: &str) -> Result<TcpStream, String> {
    let fail = |e: io::Error| format!("Cannot reach the computer at {address}: {e}");
    let addr = (address.to_socket_addrs().map_err(fail)?.next())
        .ok_or_else(|| format!("Cannot reach the computer at {address}"))?;
    let s = TcpStream::connect_timeout(&addr, Duration::from_secs(3)).map_err(fail)?;
    s.set_read_timeout(Some(WAIT * 6)).map_err(fail)?;
    s.set_write_timeout(Some(WAIT)).map_err(fail)?;
    Ok(s)
}

fn lost(e: io::Error) -> String {
    format!("The link to the computer failed: {e}")
}

/// Pairs this tablet, as `name`, with the computer at `address`, or the one
/// that answers on this Wi-Fi when `address` is empty, using the code it
/// shows; keeps the pairing in `dir`.
pub fn pair(dir: &Path, address: &str, code: &str, name: &str) -> Result<Link, String> {
    let address = if address.trim().is_empty() {
        discover().ok_or(
            "No computer answered on this Wi-Fi. Type its address (the computer's IP address).",
        )?
    } else {
        with_port(address)
    };
    let mut s = connect(&address)?;
    let ask = ToComputer::Pair {
        code: code.trim().into(),
        name: name.into(),
    };
    wire::write(&mut s, &ask).map_err(lost)?;
    match wire::read(&mut s, SMALL).map_err(lost)? {
        ToTablet::Paired { token, computer } => {
            let link = Link {
                address,
                computer,
                token,
            };
            keep_link(dir, &link)?;
            Ok(link)
        }
        ToTablet::Refused(why) => Err(why),
        _ => Err(lost(io::ErrorKind::InvalidData.into())),
    }
}

/// The projects this tablet keeps, in the order the computer last listed
/// them.
pub fn kept(dir: &Path) -> Vec<Shared> {
    fs::read(projects(dir).join("index.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// The tablet file of kept project `key`.
pub fn kept_file(dir: &Path, key: &str) -> PathBuf {
    projects(dir).join(format!("{key}.{}", crate::bundle::EXTENSION))
}

/// Fetches the projects the paired computer shares that are new or changed
/// here; how many. A computer found at a new address on this Wi-Fi is
/// remembered there.
pub fn sync(dir: &Path) -> Result<usize, String> {
    let mut link = link(dir).ok_or("Not paired with a computer.")?;
    let mut s = match connect(&link.address) {
        Ok(s) => s,
        Err(e) => {
            let found = discover().ok_or(e)?;
            let s = connect(&found)?;
            link.address = found;
            keep_link(dir, &link)?;
            s
        }
    };
    let token: Token = link.token.parse()?;
    wire::write(
        &mut s,
        &ToComputer::Hello {
            client: token.client,
        },
    )
    .map_err(lost)?;
    let nonce = match wire::read(&mut s, SMALL).map_err(lost)? {
        ToTablet::Nonce(n) => n,
        ToTablet::Refused(why) => return Err(why),
        _ => return Err(lost(io::ErrorKind::InvalidData.into())),
    };
    wire::write(&mut s, &ToComputer::Proof(token.proof(&nonce))).map_err(lost)?;
    wire::write(&mut s, &ToComputer::List).map_err(lost)?;
    let listed = match wire::read(&mut s, SMALL).map_err(lost)? {
        ToTablet::List(l) => l,
        ToTablet::Refused(why) => return Err(why),
        _ => return Err(lost(io::ErrorKind::InvalidData.into())),
    };
    let mut kept = kept(dir);
    let mut fetched = 0;
    for item in &listed {
        // The key names a file here: only what the computer makes of it.
        let safe = !item.key.is_empty()
            && item.key.len() <= 64
            && item.key.chars().all(|c| c.is_ascii_alphanumeric());
        if !safe || kept.contains(item) {
            continue;
        }
        wire::write(&mut s, &ToComputer::Get(item.key.clone())).map_err(lost)?;
        // One the computer cannot pack now (its part moved) is skipped.
        let ToTablet::Bundle(bytes) = wire::read(&mut s, BUNDLE).map_err(lost)? else {
            continue;
        };
        crate::storage::write_atomic(&kept_file(dir, &item.key), &bytes)?;
        kept.retain(|k| k.key != item.key);
        kept.push(item.clone());
        fetched += 1;
    }
    // The computer's order first, then those it no longer shares.
    let order = |k: &Shared| {
        listed
            .iter()
            .position(|l| l.key == k.key)
            .unwrap_or(usize::MAX)
    };
    kept.sort_by_key(order);
    let json = serde_json::to_vec_pretty(&kept).map_err(|e| e.to_string())?;
    crate::storage::write_atomic(&projects(dir).join("index.json"), &json)?;
    Ok(fetched)
}

// ---- Windows Firewall (LIO-485) ----
//
// Windows lets nothing in to an app until a rule allows it. The first time
// the app listens Windows asks "Allow access?", and a prompt nobody answers
// leaves inbound Block rules for the exe. Then a tablet can't find, pair or
// sync, with no error anywhere. Reading the rules needs no admin rights;
// changing them does, so the fix runs elevated (one Windows "Yes").

/// Rules for this app, on every network the computer is on now:
/// "blocked", "closed" (nothing allows it) or "open".
#[cfg(windows)]
const FIREWALL_CHECK: &str = r#"
$on = Get-NetFirewallProfile -PolicyStore ActiveStore | ? Enabled -eq 'True' | % { "$($_.Name)" }
$need = @(Get-NetConnectionProfile | % { "$($_.NetworkCategory)" -replace 'DomainAuthenticated','Domain' } | ? { $on -contains $_ })
$mine = @(Get-NetFirewallApplicationFilter -PolicyStore ActiveStore | ? { [Environment]::ExpandEnvironmentVariables("$($_.Program)") -eq $p } | Get-NetFirewallRule | ? { "$($_.Enabled)" -eq 'True' -and "$($_.Direction)" -eq 'Inbound' })
if (-not $need) { 'open' }
elseif ($mine | ? { "$($_.Action)" -eq 'Block' }) { 'blocked' }
elseif ($need | ? { $n = $_; -not ($mine | ? { "$($_.Action)" -eq 'Allow' -and ("$($_.Profile)" -eq 'Any' -or "$($_.Profile)" -match $n) }) }) { 'closed' }
else { 'open' }
"#;

/// Run as admin: drops this exe's inbound Block rules and allows it port
/// `PORT` from the local network, on any network type.
// ponytail: one rule pair for the group, so the app run from a new path
// replaces the old path's rules rather than piling them up.
#[cfg(windows)]
const FIREWALL_FIX: &str = r#"
$ErrorActionPreference = 'Stop'
$g = 'AXIS CAM shop tablet'
Get-NetFirewallApplicationFilter | ? { [Environment]::ExpandEnvironmentVariables("$($_.Program)") -eq $p } | Get-NetFirewallRule | ? { "$($_.Direction)" -eq 'Inbound' -and "$($_.Action)" -eq 'Block' } | Remove-NetFirewallRule
Get-NetFirewallRule -Group $g -EA SilentlyContinue | Remove-NetFirewallRule
foreach ($t in 'TCP', 'UDP') { New-NetFirewallRule -DisplayName "$g ($t)" -Group $g -Direction Inbound -Action Allow -Program $p -Protocol $t -LocalPort $port -RemoteAddress LocalSubnet -Profile Any | Out-Null }
"#;

/// `script` with `$p` (the exe) and `$port` set.
#[cfg(windows)]
fn for_exe(exe: &Path, script: &str) -> String {
    let p = exe.display().to_string().replace('\'', "''");
    format!("$p = '{p}'\n$port = {PORT}\n{script}")
}

/// Runs a PowerShell script with no console window.
#[cfg(windows)]
fn powershell(script: &str) -> io::Result<std::process::Output> {
    use std::os::windows::process::CommandExt;
    let script = encoded(script);
    std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &script])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
}

/// A script as PowerShell's -EncodedCommand takes it: UTF-16LE, base64.
#[cfg(windows)]
fn encoded(script: &str) -> String {
    use base64::Engine;
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(windows)]
fn firewall_state(exe: &Path) -> String {
    powershell(&for_exe(exe, FIREWALL_CHECK))
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default()
}

/// Why Windows Firewall keeps a tablet on the Wi-Fi from reaching this app,
/// or None (also when it can't tell, and off Windows).
pub fn firewall_problem() -> Option<String> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe().ok()?;
        let name = exe.file_name()?.to_string_lossy().into_owned();
        match firewall_state(&exe).as_str() {
            "blocked" => Some(format!(
                "Windows Firewall blocks this app ({name}), so the tablet can't find or reach it. Windows made Block rules when its \"Allow access?\" question went unanswered."
            )),
            "closed" => Some(format!(
                "Windows Firewall doesn't let the tablet in to this app ({name}) on every network this computer is on."
            )),
            _ => None,
        }
    }
    #[cfg(not(windows))]
    None
}

/// Asks Windows (one "Yes") to let tablets on the local network in, then
/// checks again: what still stands in the way, or None.
pub fn open_firewall() -> Option<String> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe().ok()?;
        let fix = encoded(&for_exe(&exe, FIREWALL_FIX));
        let run = format!(
            "$x = Start-Process powershell -Verb RunAs -Wait -PassThru -WindowStyle Hidden -ArgumentList '-NoProfile','-NonInteractive','-EncodedCommand','{fix}'\nexit $x.ExitCode"
        );
        let done = powershell(&run).is_ok_and(|o| o.status.success());
        firewall_problem().map(|why| {
            if done {
                why
            } else {
                format!("Windows Firewall was not changed (Windows didn't get a Yes). {why}")
            }
        })
    }
    #[cfg(not(windows))]
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(path: &Path, title: &str, part: &Path) {
        let mut doc = Document::sample();
        doc.title = title.into();
        doc.file = Some(crate::model::ModelFile {
            name: "block.stl".into(),
            path: part.into(),
            format: "stl".into(),
            bytes: 19,
            thickness: None,
        });
        crate::storage::save(path, &doc).unwrap();
    }

    #[test]
    fn projects_reach_a_paired_tablet_new_or_changed() {
        let mac = tempfile::tempdir().unwrap();
        let state = mac.path().join("workspace.json");
        let part = mac.path().join("block.stl");
        fs::write(&part, b"solid x\nendsolid x\n").unwrap();
        project(&state, "Open job", &part);
        let other = mac.path().join("Bracket.axis.json");
        project(&other, "Bracket", &part);
        let settings = crate::settings::Settings {
            recent_projects: vec![other.clone(), mac.path().join("gone.axis.json")],
            ..Default::default()
        };
        crate::settings::save(&crate::settings::path(&state), &settings).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        serve_on(listener, state.clone(), "Mac".into());

        let tablet = tempfile::tempdir().unwrap();
        let dir = tablet.path();
        assert_eq!(sync(dir).unwrap_err(), "Not paired with a computer.");
        let why = pair(dir, &address, "000000", "Shop tablet").unwrap_err();
        assert!(
            why.contains("Pair a tablet") && !why.contains("uc100"),
            "{why}"
        );
        let code = new_code(&state).unwrap();
        let wrong = if code == "000000" { "111111" } else { "000000" };
        let why = pair(dir, &address, wrong, "Shop tablet").unwrap_err();
        assert!(why.contains("Wrong pairing code"), "{why}");
        let link = pair(dir, &address, &code, "Shop tablet").unwrap();
        assert_eq!(link.computer, "Mac");

        // Both arrive, the open one first, each with its part.
        assert_eq!(sync(dir).unwrap(), 2);
        let titles: Vec<_> = kept(dir).into_iter().map(|k| k.title).collect();
        assert_eq!(titles, ["Open job", "Bracket"]);
        let open = &kept(dir)[0];
        assert_eq!(open.key, "open");
        let unpacked = tempfile::tempdir().unwrap();
        let (doc, _, _) = crate::bundle::open(&kept_file(dir, &open.key), unpacked.path()).unwrap();
        assert_eq!(doc.title, "Open job");
        assert_eq!(
            fs::read(doc.file.unwrap().path).unwrap(),
            b"solid x\nendsolid x\n"
        );

        // Unchanged: nothing fetched. Changed: only that one.
        assert_eq!(sync(dir).unwrap(), 0);
        project(&other, "Bracket v2", &part);
        assert_eq!(sync(dir).unwrap(), 1);
        assert_eq!(kept(dir)[1].title, "Bracket v2");

        // Shared no more, or the computer off: the copies stay.
        fs::remove_file(&state).unwrap();
        fs::remove_file(&other).unwrap();
        assert_eq!(sync(dir).unwrap(), 0);
        assert_eq!(kept(dir).len(), 2);
        keep_link(
            dir,
            &Link {
                address: "127.0.0.1:1".into(),
                ..link
            },
        )
        .unwrap();
        assert!(sync(dir).is_err());
        assert_eq!(kept(dir).len(), 2);
    }

    #[test]
    fn a_forged_tablet_is_refused() {
        let mac = tempfile::tempdir().unwrap();
        let state = mac.path().join("workspace.json");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        serve_on(listener, state, "Mac".into());
        let tablet = tempfile::tempdir().unwrap();
        let forged = Link {
            address,
            computer: "Mac".into(),
            token: format!("1:{}", "ab".repeat(32)),
        };
        keep_link(tablet.path(), &forged).unwrap();
        assert_eq!(sync(tablet.path()).unwrap_err(), NOT_PAIRED);
    }

    /// Not a check: shares the fixture block (Face and Drill on a UC100
    /// machine, `run::tablet_fixture`) on port 7111 for `AXIS_SERVE_MINUTES`
    /// (default 20), for testing a tablet against it (axis-shop's live test,
    /// the emulator at 10.0.2.2). The machine profile is
    /// `AXIS_SERVE_PROFILE`, by default the driver's template. Prints the
    /// pairing code.
    #[test]
    #[ignore]
    fn serve_the_fixture_to_a_tablet() {
        let mac = tempfile::tempdir().unwrap();
        let profile = match std::env::var("AXIS_SERVE_PROFILE") {
            Ok(p) => PathBuf::from(p),
            Err(_) => {
                let p = mac.path().join("template.toml");
                fs::write(&p, uc100::config::TEMPLATE).unwrap();
                p
            }
        };
        let (doc, _) = crate::run::tablet_fixture(&profile);
        let state = mac.path().join("workspace.json");
        crate::storage::save(&state, &doc).unwrap();
        println!("Pairing code: {}", new_code(&state).unwrap());
        serve(state, "Fixture computer".into());
        let minutes = std::env::var("AXIS_SERVE_MINUTES").map_or(20, |m| m.parse().unwrap());
        std::thread::sleep(Duration::from_secs(60 * minutes));
    }

    /// The TypeScript tablet app (axis-shop `src/computer/wire.test.ts`)
    /// reads and writes these exact bytes: a change here must change it too.
    #[test]
    fn the_messages_are_the_bytes_axis_shop_reads() {
        let hex = |m: Vec<u8>| self::hex(&m[4..]);
        let count = |from: u8, up: bool| -> [u8; 32] {
            std::array::from_fn(|i| if up { from + i as u8 } else { from - i as u8 })
        };
        let to_computer = [
            (
                ToComputer::Pair {
                    code: "123456".into(),
                    name: "Shop tablet".into(),
                },
                "00063132333435360b53686f70207461626c6574",
            ),
            (ToComputer::Hello { client: 300 }, "01ac02"),
            (
                ToComputer::Proof(count(0, true)),
                "02000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            ),
            (ToComputer::List, "03"),
            (ToComputer::Get("ab12".into()), "040461623132"),
        ];
        for (m, bytes) in to_computer {
            assert_eq!(hex(wire::frame(&m)), bytes);
        }
        let paired = format!("0042373a{}034d6163", "6162".repeat(32));
        let to_tablet = [
            (
                ToTablet::Paired {
                    token: format!("7:{}", "ab".repeat(32)),
                    computer: "Mac".into(),
                },
                paired.as_str(),
            ),
            (ToTablet::Refused("no".into()), "01026e6f"),
            (
                ToTablet::Nonce(count(31, false)),
                "021f1e1d1c1b1a191817161514131211100f0e0d0c0b0a09080706050403020100",
            ),
            (
                ToTablet::List(vec![Shared {
                    key: "open".into(),
                    title: "Block".into(),
                    version: "0011223344556677".into(),
                }]),
                "0301046f70656e05426c6f636b1030303131323233333434353536363737",
            ),
            (ToTablet::Bundle(vec![1, 2, 3]), "0403010203"),
        ];
        for (m, bytes) in to_tablet {
            assert_eq!(hex(wire::frame(&m)), bytes);
        }
    }

    #[test]
    fn a_typed_address_gets_the_port() {
        assert_eq!(with_port(" 10.0.2.2 "), "10.0.2.2:7111");
        assert_eq!(with_port("10.0.2.2:9000"), "10.0.2.2:9000");
        assert_eq!(with_port("mac.local"), "mac.local:7111");
    }

    /// An exe no rule names is shut out, like the app before anyone
    /// answered Windows's prompt (LIO-485). A path with a quote in it still
    /// makes a script that runs.
    #[cfg(windows)]
    #[test]
    fn the_firewall_shuts_out_an_app_no_rule_allows() {
        let exe = Path::new(r"C:\nowhere\it's axis_cam.exe");
        assert_eq!(firewall_state(exe), "closed");
    }
}
