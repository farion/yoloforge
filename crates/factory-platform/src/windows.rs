//! Native Windows ACLs and handle-based paths; never emulate Unix permissions.
use anyhow::{ensure, Result};
use std::{
    ffi::c_void,
    fs,
    os::windows::{ffi::OsStrExt, io::AsRawHandle},
    path::{Path, PathBuf},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, LocalFree},
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            GetNamedSecurityInfoW, SetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
        },
        EqualSid, GetAce, GetSecurityDescriptorDacl, GetSecurityDescriptorOwner,
        GetTokenInformation, TokenUser, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL,
        DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR, TOKEN_QUERY, TOKEN_USER,
    },
    Storage::FileSystem::{
        CreateDirectoryW, GetFinalPathNameByHandleW, MoveFileExW, MOVEFILE_WRITE_THROUGH,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

struct Descriptor(PSECURITY_DESCRIPTOR);
impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}

fn descriptor() -> Result<Descriptor> {
    let buffer = current_user()?;
    let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
    let mut sid_text = ptr::null_mut();
    ensure!(
        unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid_text) } != 0,
        "Cannot resolve runtime user SID"
    );
    let mut length = 0;
    unsafe {
        while *sid_text.add(length) != 0 {
            length += 1;
        }
    }
    let sid = String::from_utf16(unsafe { std::slice::from_raw_parts(sid_text, length) });
    unsafe {
        LocalFree(sid_text.cast());
    }
    let sid = sid?;
    // Pin ownership explicitly: elevated Windows tokens otherwise default to
    // the Administrators group. Only this user receives inherited full access.
    let text: Vec<u16> = format!("O:{sid}D:P(A;OICI;FA;;;{sid})")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut sd = ptr::null_mut();
    ensure!(
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                text.as_ptr(),
                SDDL_REVISION_1,
                &mut sd,
                ptr::null_mut(),
            )
        } != 0,
        "Cannot create private Windows ACL: {}",
        std::io::Error::last_os_error()
    );
    Ok(Descriptor(sd))
}

pub(super) fn create_private_dir(path: &Path) -> Result<()> {
    let sd = descriptor()?;
    let attrs = windows_sys::Win32::Security::SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<windows_sys::Win32::Security::SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: 0,
    };
    if unsafe { CreateDirectoryW(wide(path).as_ptr(), &attrs) } == 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(error.into());
        }
    }
    ensure_private(path)
}

pub(super) fn restrict(path: &Path) -> Result<()> {
    let sd = descriptor()?;
    let mut present = 0;
    let mut defaulted = 0;
    let mut acl = ptr::null_mut();
    let mut owner = ptr::null_mut();
    ensure!(
        unsafe { GetSecurityDescriptorDacl(sd.0, &mut present, &mut acl, &mut defaulted) } != 0
            && present != 0,
        "Cannot read private ACL"
    );
    ensure!(
        unsafe { GetSecurityDescriptorOwner(sd.0, &mut owner, &mut defaulted) } != 0,
        "Cannot read private owner"
    );
    let status = unsafe {
        SetNamedSecurityInfoW(
            wide(path).as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION
                | DACL_SECURITY_INFORMATION
                | PROTECTED_DACL_SECURITY_INFORMATION,
            owner,
            ptr::null_mut(),
            acl,
            ptr::null(),
        )
    };
    ensure!(
        status == 0,
        "Cannot protect Windows runtime path: {}",
        std::io::Error::from_raw_os_error(status as i32)
    );
    ensure_private(path)
}

pub(super) fn ensure_private(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        !metadata.file_type().is_symlink(),
        "Runtime path is a symlink"
    );
    // Only this user (or Owner Rights) may have access, and ownership must match.
    // Existing broad ACLs are rejected rather than silently taking ownership.
    let mut owner = ptr::null_mut();
    let mut acl: *mut ACL = ptr::null_mut();
    let mut sd = ptr::null_mut();
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide(path).as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut acl,
            ptr::null_mut(),
            &mut sd,
        )
    };
    ensure!(status == 0, "Cannot inspect Windows runtime ACL");
    let _sd = Descriptor(sd);
    ensure!(
        !owner.is_null() && !acl.is_null(),
        "Runtime path has no private ACL"
    );
    let buffer = current_user()?;
    let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
    ensure!(
        unsafe { EqualSid(owner, user.User.Sid) } != 0,
        "Runtime path belongs to another user"
    );
    // S-1-3-4 (Owner Rights) binary SID, aligned for Win32.
    let owner_rights: [u32; 3] = [0x00000101, 0x03000000, 4];
    let mut grants = 0;
    for index in 0..unsafe { (*acl).AceCount } as u32 {
        let mut ace: *mut c_void = ptr::null_mut();
        ensure!(
            unsafe { GetAce(acl, index, &mut ace) } != 0,
            "Cannot inspect runtime ACE"
        );
        let header = unsafe { &*ace.cast::<ACE_HEADER>() };
        ensure!(header.AceType == 0, "Unexpected runtime ACL entry");
        let allowed = unsafe { &*ace.cast::<ACCESS_ALLOWED_ACE>() };
        let sid = ptr::addr_of!(allowed.SidStart).cast_mut().cast();
        ensure!(
            unsafe { EqualSid(sid, owner_rights.as_ptr().cast_mut().cast()) } != 0
                || unsafe { EqualSid(sid, user.User.Sid) } != 0,
            "Runtime path is accessible to other users"
        );
        grants += 1;
    }
    ensure!(grants > 0, "Runtime ACL grants no owner access");
    Ok(())
}

fn current_user() -> Result<Vec<usize>> {
    let mut token = ptr::null_mut();
    ensure!(
        unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } != 0,
        "Cannot inspect runtime owner"
    );
    let mut required = 0;
    unsafe {
        GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut required);
    }
    let mut buffer = vec![0usize; (required as usize).div_ceil(std::mem::size_of::<usize>())];
    let success = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    };
    unsafe {
        CloseHandle(token);
    }
    ensure!(success != 0, "Cannot read runtime owner");
    Ok(buffer)
}

pub(super) fn opened_path(file: &fs::File) -> Result<PathBuf> {
    let handle = file.as_raw_handle();
    let length = unsafe { GetFinalPathNameByHandleW(handle, ptr::null_mut(), 0, 0) };
    ensure!(length > 0, "Cannot resolve opened Windows file");
    let mut buffer = vec![0u16; length as usize + 1];
    let count =
        unsafe { GetFinalPathNameByHandleW(handle, buffer.as_mut_ptr(), buffer.len() as u32, 0) };
    ensure!(
        count > 0 && count < buffer.len() as u32,
        "Opened file path changed"
    );
    use std::os::windows::ffi::OsStringExt;
    Ok(fs::canonicalize(std::ffi::OsString::from_wide(
        &buffer[..count as usize],
    ))?)
}

pub(super) fn rename_noreplace(source: &Path, destination: &Path) -> Result<()> {
    ensure!(
        unsafe {
            MoveFileExW(
                wide(source).as_ptr(),
                wide(destination).as_ptr(),
                MOVEFILE_WRITE_THROUGH,
            )
        } != 0,
        "Cannot publish project without replacement: {}",
        std::io::Error::last_os_error()
    );
    Ok(())
}
