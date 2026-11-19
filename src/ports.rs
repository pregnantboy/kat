use std::ffi::CStr;

use libproc::libproc::bsd_info::BSDInfo;
use libproc::libproc::file_info::{pidfdinfo, ListFDs, ProcFDType};
use libproc::libproc::net_info::{SocketFDInfo, SocketInfoKind};
use libproc::libproc::proc_pid::{listpidinfo, pidinfo, pidpath};
use libproc::processes::{pids_by_type, ProcFilter};

const TSI_S_LISTEN: i32 = 1;
const INTERPRETERS: &[&str] = &["node", "python", "ruby", "bun", "deno", "java", "php", "perl"];

#[derive(Clone, Debug, PartialEq)]
pub struct Port {
    pub port: u16,
    pub pid: i32,
    pub uid: u32,
    pub exe: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
}

/// Listening TCP ports owned by processes we can inspect (our own user's), sorted by port then pid.
pub fn list() -> Vec<Port> {
    let mut found: Vec<(u16, i32, u32)> = Vec::new();
    for pid in pids_by_type(ProcFilter::All).unwrap_or_default() {
        let pid = pid as i32;
        let Ok(info) = pidinfo::<BSDInfo>(pid, 0) else { continue };
        let Ok(fds) = listpidinfo::<ListFDs>(pid, info.pbi_nfiles as usize) else { continue };
        for fd in fds {
            if !matches!(fd.proc_fdtype.into(), ProcFDType::Socket) {
                continue;
            }
            let Ok(s) = pidfdinfo::<SocketFDInfo>(pid, fd.proc_fd) else { continue };
            if !matches!(s.psi.soi_kind.into(), SocketInfoKind::Tcp) {
                continue;
            }
            let tcp = unsafe { s.psi.soi_proto.pri_tcp };
            if tcp.tcpsi_state == TSI_S_LISTEN {
                found.push((u16::from_be(tcp.tcpsi_ini.insi_lport as u16), pid, info.pbi_uid));
            }
        }
    }
    found.sort();
    found.dedup(); // IPv4 + IPv6 listeners on the same port
    found
        .into_iter()
        .map(|(port, pid, uid)| Port {
            port,
            pid,
            uid,
            exe: pidpath(pid).unwrap_or_default(),
            args: args(pid).unwrap_or_default(),
            cwd: cwd(pid),
        })
        .collect()
}

pub fn kill(pid: i32, force: bool) -> bool {
    // 0 and -1 would signal a whole process group or every process we own.
    if pid <= 0 {
        return false;
    }
    unsafe { libc::kill(pid, if force { libc::SIGKILL } else { libc::SIGTERM }) == 0 }
}

/// Signal 0 only checks existence; EPERM means it exists but isn't ours.
pub fn alive(pid: i32) -> bool {
    pid > 0 && unsafe { libc::kill(pid, 0) == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) }
}

pub fn my_uid() -> u32 {
    unsafe { libc::getuid() }
}

fn cwd(pid: i32) -> Option<String> {
    let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as i32;
    let rc = unsafe {
        libc::proc_pidinfo(pid, libc::PROC_PIDVNODEPATHINFO, 0, &mut info as *mut _ as *mut _, size)
    };
    if rc != size {
        return None;
    }
    let path = unsafe { CStr::from_ptr(info.pvi_cdir.vip_path.as_ptr() as *const libc::c_char) };
    Some(path.to_string_lossy().into_owned())
}

// KERN_PROCARGS2 buffer: argc (i32), the exec path, NUL padding, then argc NUL-separated args.
fn args(pid: i32) -> Option<Vec<String>> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut size: libc::size_t = 0;
    let null = std::ptr::null_mut();
    if unsafe { libc::sysctl(mib.as_mut_ptr(), 3, null, &mut size, null, 0) } != 0 {
        return None;
    }
    let mut buf = vec![0u8; size];
    if unsafe { libc::sysctl(mib.as_mut_ptr(), 3, buf.as_mut_ptr() as *mut _, &mut size, null, 0) } != 0 {
        return None;
    }
    buf.truncate(size);
    let argc = i32::from_ne_bytes(buf.get(..4)?.try_into().ok()?) as usize;
    let rest = &buf[4..];
    let rest = &rest[rest.iter().position(|&b| b == 0)?..];
    let rest = &rest[rest.iter().position(|&b| b != 0)?..];
    Some(rest.split(|&b| b == 0).take(argc).map(|s| String::from_utf8_lossy(s).into_owned()).collect())
}

fn file_name(path: &str) -> &str {
    path.trim().rsplit('/').next().unwrap_or(path)
}

/// The executable's file name, e.g. `node`.
pub fn process(p: &Port) -> &str {
    file_name(&p.exe)
}

/// Case-insensitive: python3 re-executes itself as `.../Python.app/Contents/MacOS/Python`.
fn is_interpreter(cmd: &str) -> bool {
    let cmd = cmd.to_lowercase();
    INTERPRETERS.iter().any(|i| {
        cmd.strip_prefix(i).is_some_and(|v| v.chars().all(|c| c.is_ascii_digit() || c == '.'))
    })
}

/// What a person would call this process: the script an interpreter runs, the Mac app it belongs to, or its command.
pub fn name(p: &Port) -> String {
    let cmd = p.args.first().map(|a| file_name(a)).unwrap_or_default();
    // Interpreters come first: Xcode's and Homebrew's python3 live inside a Python.app bundle.
    if !is_interpreter(cmd) {
        if let Some(app) = p.exe.split('/').find_map(|c| c.strip_suffix(".app")) {
            return app.to_string();
        }
        return if cmd.is_empty() { file_name(&p.exe) } else { cmd }.to_string();
    }
    let mut rest = p.args[1..].iter().filter(|a| !a.starts_with('-'));
    let Some(script) = rest.next() else { return cmd.to_string() };
    let mut name = file_name(script).to_string();
    if let Some(sub) = rest.next().filter(|a| !a.contains('/')) {
        name.push(' ');
        name.push_str(sub);
    }
    name
}

pub fn project(p: &Port, home: &str) -> Option<String> {
    let cwd = p.cwd.as_deref().filter(|c| !c.is_empty() && *c != "/")?;
    Some(match cwd.strip_prefix(home) {
        Some(rest) if !home.is_empty() && (rest.is_empty() || rest.starts_with('/')) => format!("~{rest}"),
        _ => cwd.to_string(),
    })
}

/// The badge for a port whose process must not be stopped, or None if it can be.
pub fn protection(p: &Port, my_uid: u32, terminal_pid: i32, self_pid: i32) -> Option<&'static str> {
    if p.pid == terminal_pid {
        Some("this terminal")
    } else if p.pid == self_pid {
        Some("killer")
    } else if p.uid != my_uid {
        Some("other user")
    } else if p.exe.contains("OrbStack") || p.exe.contains("Docker") {
        // These own every forwarded container port; killing one stops every container.
        Some("docker")
    } else if p.exe.starts_with("/System/") || (p.exe.starts_with("/usr/") && !p.exe.starts_with("/usr/local/")) {
        Some("system")
    } else {
        None
    }
}

pub fn matches(p: &Port, needle: &str, home: &str) -> bool {
    let needle = needle.trim_start_matches(':').to_lowercase();
    p.port.to_string().contains(&needle)
        || name(p).to_lowercase().contains(&needle)
        || project(p, home).is_some_and(|d| d.to_lowercase().contains(&needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(port: u16, pid: i32, exe: &str, args: &[&str], cwd: Option<&str>) -> Port {
        Port {
            port,
            pid,
            uid: 502,
            exe: exe.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            cwd: cwd.map(String::from),
        }
    }

    const NODE: &str = "/Users/ian/.nvm/versions/node/v24.22.1/bin/node";

    #[test]
    fn names_from_real_command_lines() {
        let helper = "/Applications/OrbStack.app/Contents/Frameworks/OrbStack Helper.app/Contents/MacOS/OrbStack Helper";
        assert_eq!(name(&port(5432, 1, helper, &[helper, "vm"], None)), "OrbStack");
        let cc = "/System/Library/CoreServices/ControlCenter.app/Contents/MacOS/ControlCenter";
        assert_eq!(name(&port(5000, 1, cc, &[cc], None)), "ControlCenter");
        assert_eq!(name(&port(3000, 1, NODE, &["next-server (v16.4.1) "], None)), "next-server (v16.4.1)");
        assert_eq!(name(&port(5158, 1, NODE, &["node", "test/mock-singpass/server.mjs"], None)), "server.mjs");
        assert_eq!(name(&port(3001, 1, NODE, &[NODE, "/x/node_modules/.bin/next", "dev"], None)), "next dev");
        let xcode_py = "/Applications/Xcode.app/Contents/Developer/Library/Frameworks/Python3.framework/Versions/3.9/Resources/Python.app/Contents/MacOS/Python";
        assert_eq!(name(&port(8764, 1, xcode_py, &[xcode_py, "-m", "http.server", "8764"], None)), "http.server 8764");
        let py = "/opt/homebrew/bin/python3.12";
        assert_eq!(name(&port(8765, 1, py, &["python3.12", "-m", "http.server", "8765"], None)), "http.server 8765");
        let codex = "/Users/ian/.codex/bin/codex";
        assert_eq!(name(&port(50892, 1, codex, &["codex", "--no-daemon", "-c", "x=1"], None)), "codex");
        assert_eq!(name(&port(1, 1, "/usr/libexec/rapportd", &[], None)), "rapportd");
    }

    #[test]
    fn project_shortens_home_and_skips_root() {
        let p = port(3000, 1, NODE, &[], Some("/Users/ian/code/shop"));
        assert_eq!(project(&p, "/Users/ian").as_deref(), Some("~/code/shop"));
        assert_eq!(project(&port(1, 1, NODE, &[], Some("/")), "/Users/ian"), None);
        assert_eq!(project(&port(1, 1, NODE, &[], None), "/Users/ian"), None);
        assert_eq!(project(&port(1, 1, NODE, &[], Some("/Users/iansmith/x")), "/Users/ian").as_deref(), Some("/Users/iansmith/x"));
        assert_eq!(project(&port(1, 1, NODE, &[], Some("/Users/ian")), "/Users/ian").as_deref(), Some("~"));
        assert_eq!(project(&port(1, 1, NODE, &[], Some("")), "/Users/ian"), None);
    }

    #[test]
    fn kill_refuses_non_positive_pids() {
        assert!(!kill(0, false));
        assert!(!kill(-1, false));
    }

    #[test]
    fn alive_checks_existence_without_signalling() {
        assert!(alive(std::process::id() as i32));
        assert!(!alive(0) && !alive(-1));
    }

    #[test]
    fn protection_badges() {
        let me = 502;
        let ok = port(3000, 10, NODE, &[], None);
        assert_eq!(protection(&ok, me, 4, 99), None);
        assert_eq!(protection(&port(1, 4, NODE, &[], None), me, 4, 99), Some("this terminal"));
        assert_eq!(protection(&port(1, 99, NODE, &[], None), me, 4, 99), Some("killer"));
        assert_eq!(protection(&Port { uid: 0, ..ok.clone() }, me, 4, 99), Some("other user"));
        let orb = "/Applications/OrbStack.app/Contents/MacOS/OrbStack";
        assert_eq!(protection(&port(5432, 10, orb, &[], None), me, 4, 99), Some("docker"));
        assert_eq!(protection(&port(1, 10, "/usr/libexec/rapportd", &[], None), me, 4, 99), Some("system"));
        let cc = "/System/Library/CoreServices/ControlCenter.app/Contents/MacOS/ControlCenter";
        assert_eq!(protection(&port(5000, 10, cc, &[], None), me, 4, 99), Some("system"));
        assert_eq!(protection(&port(5432, 10, "/usr/local/bin/postgres", &[], None), me, 4, 99), None);
    }

    #[test]
    fn filter_matches_port_name_and_project() {
        let p = port(3000, 1, NODE, &["next-server (v16.4.1)"], Some("/Users/ian/code/shop"));
        for needle in ["3000", ":3000", "NEXT", "shop"] {
            assert!(matches(&p, needle, "/Users/ian"), "{needle}");
        }
        assert!(!matches(&p, "5432", "/Users/ian"));
    }
}

/// Spawns a real server and stops it. Run with `cargo test -- --ignored`.
#[cfg(test)]
mod live {
    use super::*;
    use std::process::{Command, Stdio};
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    #[ignore]
    fn finds_names_and_stops_a_real_server() {
        let mut child = Command::new("python3")
            .args(["-m", "http.server", "8765"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        sleep(Duration::from_secs(1));
        let all = list();
        let p = all.iter().find(|p| p.port == 8765).expect(":8765 should be listed");
        assert_eq!(name(p), "http.server 8765");
        assert_eq!(protection(p, my_uid(), -1, -1), None);
        assert!(kill(p.pid, false));
        sleep(Duration::from_millis(500));
        assert!(child.try_wait().unwrap().is_some(), "server should have exited");
        assert!(!list().iter().any(|p| p.port == 8765));
    }
}
