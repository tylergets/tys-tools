//! Resolving polkit's identities (users and groups) to the accounts someone can log in as,
//! through the C library so NSS sources (LDAP, systemd-homed, ...) are honored.

use std::ffi::{c_char, c_int, CStr, CString};
use std::mem::MaybeUninit;
use std::ptr;

use crate::polkit::Identity;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub uid: u32,
    pub name: String,
    /// From the GECOS field, if any.
    pub full_name: String,
}

impl User {
    /// What to show in a chooser: `Full Name (name)`, or just the name.
    pub fn display(&self) -> String {
        if self.full_name.is_empty() || self.full_name == self.name {
            self.name.clone()
        } else {
            format!("{} ({})", self.full_name, self.name)
        }
    }
}

pub fn current_uid() -> u32 {
    // SAFETY: getuid has no preconditions.
    unsafe { libc::getuid() }
}

/// Everyone in `identities` who could authenticate, as polkit listed them, except that the
/// current user comes first. Groups contribute their members.
pub fn candidates(identities: &[Identity]) -> Vec<User> {
    let mut users: Vec<User> = Vec::new();
    let mut add = |user: User| {
        if !users.iter().any(|u| u.uid == user.uid) {
            users.push(user);
        }
    };
    for identity in identities {
        match identity {
            Identity::User(uid) => {
                if let Some(user) = by_uid(*uid) {
                    add(user);
                }
            }
            Identity::Group(gid) => {
                for name in group_members(*gid) {
                    if let Some(user) = by_name(&name) {
                        add(user);
                    }
                }
            }
            Identity::Other(kind) => eprintln!("warning: ignoring unsupported identity {kind}"),
        }
    }
    let me = current_uid();
    if let Some(index) = users.iter().position(|u| u.uid == me) {
        let user = users.remove(index);
        users.insert(0, user);
    }
    users
}

pub fn by_uid(uid: u32) -> Option<User> {
    // SAFETY: getpwuid_r writes within the buffer it is given; see `lookup_passwd`.
    lookup_passwd(|pwd, buf, len, result| unsafe { libc::getpwuid_r(uid, pwd, buf, len, result) })
}

pub fn by_name(name: &str) -> Option<User> {
    let name = CString::new(name).ok()?;
    // SAFETY: as for `by_uid`; `name` outlives the call.
    lookup_passwd(|pwd, buf, len, result| unsafe {
        libc::getpwnam_r(name.as_ptr(), pwd, buf, len, result)
    })
}

/// Run one of the `getpw*_r` calls, growing the string buffer until it fits.
fn lookup_passwd(
    call: impl Fn(*mut libc::passwd, *mut c_char, usize, *mut *mut libc::passwd) -> c_int,
) -> Option<User> {
    let mut buf: Vec<c_char> = vec![0; 4096];
    loop {
        let mut pwd = MaybeUninit::<libc::passwd>::uninit();
        let mut result: *mut libc::passwd = ptr::null_mut();
        let rc = call(pwd.as_mut_ptr(), buf.as_mut_ptr(), buf.len(), &mut result);
        if rc == libc::ERANGE {
            buf.resize(buf.len() * 2, 0);
            continue;
        }
        if rc != 0 || result.is_null() {
            return None;
        }
        // SAFETY: on success the C library filled `pwd`, and its string fields point into
        // `buf`, which is alive until we return.
        let pwd = unsafe { pwd.assume_init() };
        let name = unsafe { c_string(pwd.pw_name) };
        let gecos = unsafe { c_string(pwd.pw_gecos) };
        return Some(User {
            uid: pwd.pw_uid,
            name,
            full_name: gecos.split(',').next().unwrap_or("").trim().to_string(),
        });
    }
}

/// The names of a group's members (its `gr_mem` list).
pub fn group_members(gid: u32) -> Vec<String> {
    let mut buf: Vec<c_char> = vec![0; 4096];
    loop {
        let mut grp = MaybeUninit::<libc::group>::uninit();
        let mut result: *mut libc::group = ptr::null_mut();
        // SAFETY: getgrgid_r writes within `buf`, whose length we pass.
        let rc = unsafe {
            libc::getgrgid_r(
                gid,
                grp.as_mut_ptr(),
                buf.as_mut_ptr(),
                buf.len(),
                &mut result,
            )
        };
        if rc == libc::ERANGE {
            buf.resize(buf.len() * 2, 0);
            continue;
        }
        if rc != 0 || result.is_null() {
            return Vec::new();
        }
        // SAFETY: as in `lookup_passwd`; `gr_mem` is a NULL-terminated array of C strings.
        let grp = unsafe { grp.assume_init() };
        let mut members = Vec::new();
        let mut entry = grp.gr_mem;
        while !entry.is_null() {
            let name = unsafe { *entry };
            if name.is_null() {
                break;
            }
            members.push(unsafe { c_string(name) });
            entry = unsafe { entry.add(1) };
        }
        return members;
    }
}

/// # Safety
/// `ptr` must be null or point to a NUL-terminated string.
unsafe fn c_string(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    CStr::from_ptr(ptr).to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_exists() {
        let root = by_uid(0).expect("uid 0");
        assert_eq!(root.name, "root");
        assert_eq!(by_name("root").unwrap().uid, 0);
    }

    #[test]
    fn current_user_first() {
        let me = current_uid();
        let users = candidates(&[Identity::User(0), Identity::User(me)]);
        assert_eq!(users.first().map(|u| u.uid), Some(me));
        assert_eq!(users.len(), if me == 0 { 1 } else { 2 });
    }

    #[test]
    fn display_name() {
        let user = User {
            uid: 1,
            name: "ty".into(),
            full_name: "Ty Lee".into(),
        };
        assert_eq!(user.display(), "Ty Lee (ty)");
    }
}
