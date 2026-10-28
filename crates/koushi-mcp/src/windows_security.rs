//! Windows local-bridge ACL boundary. SYSTEM/administrators are OS-trusted;
//! no other principal may read a connection token or connect to the pipe.
use std::ffi::c_void;
use std::io;
use std::os::windows::{ffi::OsStrExt, io::AsRawHandle};
use std::path::Path;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{CloseHandle, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo,
    SE_FILE_OBJECT, SetNamedSecurityInfoW,
};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, DACL_SECURITY_INFORMATION, EqualSid, GetAce,
    GetSecurityDescriptorDacl, GetSecurityDescriptorOwner, GetTokenInformation, IsWellKnownSid,
    OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, SECURITY_ATTRIBUTES,
    TOKEN_QUERY, TOKEN_USER, TokenUser, WinBuiltinAdministratorsSid, WinCreatorOwnerRightsSid,
    WinLocalSystemSid,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "untrusted local bridge ACL",
    )
}

/// Allocated by Windows, released with its matching allocator.
pub struct PrivateSecurity(*mut c_void);
impl Drop for PrivateSecurity {
    fn drop(&mut self) {
        // SAFETY: ConvertStringSecurityDescriptor allocated this descriptor.
        unsafe {
            LocalFree(self.0);
        }
    }
}
impl PrivateSecurity {
    pub fn new() -> io::Result<Self> {
        // Owner rights apply to the actual object owner; OI/CI propagate to
        // endpoint/grant files. D:P prevents broad inherited permissions.
        let user = current_user()?;
        let mut sid_string = null_mut();
        // SAFETY: the aligned TOKEN_USER owns its SID for this call. The
        // returned UTF-16 string is LocalAlloc memory, freed below.
        if unsafe {
            ConvertSidToStringSidW(
                (*(user.as_ptr().cast::<TOKEN_USER>())).User.Sid,
                &mut sid_string,
            )
        } == 0
        {
            return Err(denied());
        }
        let allocation = PrivateSecurity(sid_string.cast());
        let mut length = 0;
        // SAFETY: ConvertSidToStringSidW returned a NUL-terminated string.
        unsafe {
            while *sid_string.add(length) != 0 {
                length += 1;
            }
        }
        // SAFETY: the slice remains within the live Windows SID string.
        let owner = String::from_utf16(unsafe { std::slice::from_raw_parts(sid_string, length) })
            .map_err(|_| denied())?;
        drop(allocation);
        let sddl: Vec<u16> =
            format!("O:{owner}D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)\0")
                .encode_utf16()
                .collect();
        let mut descriptor = null_mut();
        // SAFETY: NUL-terminated input and valid out pointer; Windows owns no
        // references to the input after this synchronous call.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                null_mut(),
            )
        } == 0
        {
            return Err(denied());
        }
        Ok(Self(descriptor))
    }
    pub fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0,
            bInheritHandle: 0,
        }
    }
    pub fn apply(&self, path: &Path) -> io::Result<()> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut present = 0;
        let mut defaulted = 0;
        let mut acl = null_mut();
        let mut owner = null_mut();
        // SAFETY: descriptor is live; all output pointers are valid. The ACL
        // remains owned by self throughout SetNamedSecurityInfoW.
        unsafe {
            if GetSecurityDescriptorDacl(self.0, &mut present, &mut acl, &mut defaulted) == 0
                || present == 0
                || acl.is_null()
            {
                return Err(denied());
            }
            if GetSecurityDescriptorOwner(self.0, &mut owner, &mut defaulted) == 0
                || owner.is_null()
            {
                return Err(denied());
            }
            if SetNamedSecurityInfoW(
                wide.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION
                    | PROTECTED_DACL_SECURITY_INFORMATION
                    | OWNER_SECURITY_INFORMATION,
                owner,
                null_mut(),
                acl,
                null(),
            ) != 0
            {
                return Err(denied());
            }
        }
        Ok(())
    }
}

/// Validate the opened file itself, not a potentially replaced path. Default
/// or custom endpoint locations with broad/null DACLs fail closed.
pub fn validate_file(file: &std::fs::File) -> io::Result<()> {
    validate_handle(file.as_raw_handle())
}

/// Authenticate the connected server before any token/query bytes are sent.
pub fn validate_pipe(pipe: &tokio::net::windows::named_pipe::NamedPipeClient) -> io::Result<()> {
    validate_handle(pipe.as_raw_handle())
}

fn validate_handle(handle: windows_sys::Win32::Foundation::HANDLE) -> io::Result<()> {
    let mut owner = null_mut();
    let mut acl: *mut ACL = null_mut();
    let mut descriptor = null_mut();
    // SAFETY: the file handle is live and all out pointers are writable.
    if unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            &mut acl,
            null_mut(),
            &mut descriptor,
        )
    } != 0
    {
        return Err(denied());
    }
    let descriptor = PrivateSecurity(descriptor);
    let user = current_user()?;
    if owner.is_null() || acl.is_null() {
        return Err(denied());
    }
    // SAFETY: GetTokenInformation initialized a TOKEN_USER in this aligned
    // buffer. GetSecurityInfo owns owner/ACL/ACE pointers until descriptor drops.
    unsafe {
        let user_sid = (*(user.as_ptr().cast::<TOKEN_USER>())).User.Sid;
        if EqualSid(owner, user_sid) == 0 {
            return Err(denied());
        }
        for index in 0..u32::from((*acl).AceCount) {
            let mut ace = null_mut();
            if GetAce(acl, index, &mut ace) == 0 || ace.is_null() {
                return Err(denied());
            }
            match (*(ace.cast::<ACE_HEADER>())).AceType {
                0 => {
                    let allowed = &*ace.cast::<ACCESS_ALLOWED_ACE>();
                    let sid = std::ptr::addr_of!(allowed.SidStart).cast_mut().cast();
                    if EqualSid(sid, user_sid) == 0
                        && IsWellKnownSid(sid, WinCreatorOwnerRightsSid) == 0
                        && IsWellKnownSid(sid, WinLocalSystemSid) == 0
                        && IsWellKnownSid(sid, WinBuiltinAdministratorsSid) == 0
                    {
                        return Err(denied());
                    }
                }
                1 => {} // Deny ACEs cannot widen access.
                _ => return Err(denied()),
            }
        }
    }
    drop(descriptor);
    Ok(())
}

fn current_user() -> io::Result<Vec<usize>> {
    let mut token = null_mut();
    // SAFETY: process handle is valid; token is closed immediately below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(denied());
    }
    let mut size = 0;
    // SAFETY: size-only query permits a null output buffer.
    unsafe {
        GetTokenInformation(token, TokenUser, null_mut(), 0, &mut size);
    }
    let mut user = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
    // SAFETY: usize allocation is aligned for TOKEN_USER and has size bytes.
    let ok =
        unsafe { GetTokenInformation(token, TokenUser, user.as_mut_ptr().cast(), size, &mut size) };
    // SAFETY: token was returned by OpenProcessToken and is no longer used.
    unsafe {
        CloseHandle(token);
    }
    if ok == 0 || user.is_empty() {
        return Err(denied());
    }
    Ok(user)
}
