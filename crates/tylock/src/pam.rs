//! A minimal libpam binding: run one authentication with a conversation that answers
//! prompts with the password (if any) and reports PAM's messages.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::ptr;

const PAM_SUCCESS: c_int = 0;
const PAM_BUF_ERR: c_int = 5;
const PAM_CONV_ERR: c_int = 19;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;
const PAM_ERROR_MSG: c_int = 3;
const PAM_TEXT_INFO: c_int = 4;
const PAM_REFRESH_CRED: c_int = 0x0010;
/// Linux-PAM's limit on messages per conversation call.
const PAM_MAX_NUM_MSG: c_int = 32;

#[repr(C)]
struct PamHandle {
    _private: [u8; 0],
}

#[repr(C)]
struct PamMessage {
    msg_style: c_int,
    msg: *const c_char,
}

#[repr(C)]
struct PamResponse {
    resp: *mut c_char,
    resp_retcode: c_int,
}

type ConvFn = unsafe extern "C" fn(
    num_msg: c_int,
    msg: *mut *const PamMessage,
    resp: *mut *mut PamResponse,
    appdata: *mut c_void,
) -> c_int;

#[repr(C)]
struct PamConv {
    conv: ConvFn,
    appdata_ptr: *mut c_void,
}

#[link(name = "pam")]
extern "C" {
    fn pam_start(
        service: *const c_char,
        user: *const c_char,
        conv: *const PamConv,
        handle: *mut *mut PamHandle,
    ) -> c_int;
    fn pam_authenticate(handle: *mut PamHandle, flags: c_int) -> c_int;
    fn pam_setcred(handle: *mut PamHandle, flags: c_int) -> c_int;
    fn pam_end(handle: *mut PamHandle, status: c_int) -> c_int;
    fn pam_strerror(handle: *mut PamHandle, errnum: c_int) -> *const c_char;
}

/// Something PAM wants shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Info(String),
    Error(String),
}

/// A string that is wiped from memory when dropped.
#[derive(Default)]
pub struct Secret(String);

impl Secret {
    pub fn push_str(&mut self, text: &str) {
        // Grow by copying into a fresh buffer we control, so no stale copy is left behind
        // by a reallocation.
        if self.0.len() + text.len() > self.0.capacity() {
            let mut bigger = String::with_capacity((self.0.len() + text.len()).max(64) * 2);
            bigger.push_str(&self.0);
            self.clear();
            self.0 = bigger;
        }
        self.0.push_str(text);
    }

    pub fn pop(&mut self) {
        if let Some(c) = self.0.pop() {
            let len = self.0.len();
            // SAFETY: zeroing bytes past the end of the string leaves it valid UTF-8.
            unsafe { wipe(&mut self.0.as_mut_vec().spare_capacity_mut()[..c.len_utf8()]) };
            debug_assert_eq!(self.0.len(), len);
        }
    }

    pub fn clear(&mut self) {
        // SAFETY: an all-zero string is valid UTF-8, and it is truncated right after.
        unsafe { wipe_init(self.0.as_mut_vec()) };
        self.0.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn chars(&self) -> usize {
        self.0.chars().count()
    }

    /// Move the contents out, leaving this empty.
    pub fn take(&mut self) -> Secret {
        Secret(std::mem::take(&mut self.0))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.clear();
    }
}

fn wipe(bytes: &mut [std::mem::MaybeUninit<u8>]) {
    for byte in bytes {
        // SAFETY: a volatile write to memory we own, so it isn't optimized away.
        unsafe { ptr::write_volatile(byte.as_mut_ptr(), 0) };
    }
}

fn wipe_init(bytes: &mut [u8]) {
    for byte in bytes {
        // SAFETY: as in `wipe`.
        unsafe { ptr::write_volatile(byte, 0) };
    }
}

struct Conversation<'a> {
    password: Option<&'a CStr>,
    on_message: &'a mut dyn FnMut(Message),
}

unsafe extern "C" fn converse(
    num_msg: c_int,
    msg: *mut *const PamMessage,
    resp: *mut *mut PamResponse,
    appdata: *mut c_void,
) -> c_int {
    if num_msg <= 0 || num_msg > PAM_MAX_NUM_MSG || msg.is_null() || resp.is_null() {
        return PAM_CONV_ERR;
    }
    let count = num_msg as usize;
    // SAFETY: `appdata` is the `Conversation` passed to pam_start, alive for the whole
    // transaction (see `authenticate`).
    let conv = unsafe { &mut *(appdata as *mut Conversation) };
    // PAM frees the responses (and each `resp` string) with free().
    let replies =
        unsafe { libc::calloc(count, std::mem::size_of::<PamResponse>()) } as *mut PamResponse;
    if replies.is_null() {
        return PAM_BUF_ERR;
    }
    for i in 0..count {
        // SAFETY: Linux-PAM passes an array of `num_msg` message pointers.
        let message = unsafe { &**msg.add(i) };
        let text = if message.msg.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(message.msg) }
                .to_string_lossy()
                .trim()
                .to_string()
        };
        match message.msg_style {
            PAM_PROMPT_ECHO_OFF | PAM_PROMPT_ECHO_ON => {
                let Some(password) = conv.password else {
                    // Nothing to answer with (the fingerprint-only transaction): give up.
                    unsafe { free_replies(replies, count) };
                    return PAM_CONV_ERR;
                };
                let copy = unsafe { libc::strdup(password.as_ptr()) };
                if copy.is_null() {
                    unsafe { free_replies(replies, count) };
                    return PAM_BUF_ERR;
                }
                unsafe { (*replies.add(i)).resp = copy };
            }
            PAM_ERROR_MSG => (conv.on_message)(Message::Error(text)),
            PAM_TEXT_INFO => (conv.on_message)(Message::Info(text)),
            _ => {}
        }
    }
    unsafe { *resp = replies };
    PAM_SUCCESS
}

/// Free a reply array from `converse` that is not handed to PAM, wiping the answers.
unsafe fn free_replies(replies: *mut PamResponse, count: usize) {
    for i in 0..count {
        let answer = unsafe { (*replies.add(i)).resp };
        if !answer.is_null() {
            let len = unsafe { libc::strlen(answer) };
            wipe_init(unsafe { std::slice::from_raw_parts_mut(answer.cast::<u8>(), len) });
            unsafe { libc::free(answer.cast()) };
        }
    }
    unsafe { libc::free(replies.cast()) };
}

/// Authenticate `user` with PAM `service`. Prompts are answered with `password`; without
/// one (a fingerprint-only service) a prompt fails the attempt. On failure the error is
/// PAM's description of it.
pub fn authenticate(
    service: &str,
    user: &str,
    password: Option<&Secret>,
    on_message: &mut dyn FnMut(Message),
) -> Result<(), String> {
    let service = CString::new(service).map_err(|_| "invalid PAM service name".to_string())?;
    let user = CString::new(user).map_err(|_| "invalid user name".to_string())?;
    let password = match password {
        Some(secret) => Some(
            CString::new(secret.as_str().as_bytes().to_vec())
                .map_err(|_| "the password can't contain NUL".to_string())?,
        ),
        None => None,
    };
    let result = {
        let mut conversation = Conversation {
            password: password.as_deref(),
            on_message,
        };
        let conv = PamConv {
            conv: converse,
            appdata_ptr: (&mut conversation as *mut Conversation).cast(),
        };
        let mut handle: *mut PamHandle = ptr::null_mut();
        // SAFETY: valid C strings and a conversation that outlives the transaction, which
        // ends with pam_end below.
        unsafe {
            let rc = pam_start(service.as_ptr(), user.as_ptr(), &conv, &mut handle);
            if rc != PAM_SUCCESS || handle.is_null() {
                Err(format!("pam_start failed ({rc})"))
            } else {
                let rc = pam_authenticate(handle, 0);
                if rc == PAM_SUCCESS {
                    // Renew Kerberos tickets and the like, as other lockers do.
                    pam_setcred(handle, PAM_REFRESH_CRED);
                }
                let result = if rc == PAM_SUCCESS {
                    Ok(())
                } else {
                    let text = pam_strerror(handle, rc);
                    Err(if text.is_null() {
                        format!("PAM error {rc}")
                    } else {
                        CStr::from_ptr(text).to_string_lossy().into_owned()
                    })
                };
                pam_end(handle, rc);
                result
            }
        }
    };
    if let Some(password) = password {
        let mut bytes = password.into_bytes();
        wipe_init(&mut bytes);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_edits() {
        let mut secret = Secret::default();
        secret.push_str("hé");
        secret.push_str("llo");
        assert_eq!(secret.as_str(), "héllo");
        assert_eq!(secret.chars(), 5);
        secret.pop();
        assert_eq!(secret.as_str(), "héll");
        let taken = secret.take();
        assert!(secret.is_empty());
        assert_eq!(taken.as_str(), "héll");
        secret.push_str(&"x".repeat(500));
        assert_eq!(secret.chars(), 500);
        secret.clear();
        assert!(secret.is_empty());
    }
}
