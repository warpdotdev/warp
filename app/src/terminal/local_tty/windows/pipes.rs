//! Logic to open named pipes on Windows.
//!
//! Some of the code is based on the Windows Terminal code here:
//! https://github.com/microsoft/terminal/blob/93930bb3fa99d4e6986a85e7950caefb717af910/src/types/utils.cpp#L701-L713
//!
//! Adapted under the MIT License, Copyright (c) Microsoft Corporation.  See app/assets/windows/LICENSE-WINDOWS-TERMINAL.

use std::sync::LazyLock;

use windows::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows::Wdk::Storage::FileSystem::{
    NtCreateFile, FILE_CREATE, FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_PIPE_BYTE_STREAM_MODE,
    FILE_PIPE_BYTE_STREAM_TYPE, FILE_PIPE_QUEUE_OPERATION, FILE_SYNCHRONOUS_IO_NONALERT,
    NTCREATEFILE_CREATE_OPTIONS,
};
use windows::Win32::Foundation::{
    GENERIC_READ, GENERIC_WRITE, HANDLE, NTSTATUS, OBJ_CASE_INSENSITIVE, UNICODE_STRING,
};
use windows::Win32::Storage::FileSystem::{
    FILE_ACCESS_RIGHTS, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, SYNCHRONIZE,
};
use windows::Win32::System::WindowsProgramming::RtlInitUnicodeString;
use windows::Win32::System::IO::IO_STATUS_BLOCK;

use crate::terminal::local_tty::windows::ShareableHandle;

/// A handle to the device directory where we can open named pipes.
///
/// This is based on the Windows Terminal code here:
/// https://github.com/microsoft/terminal/blob/93930bb3fa99d4e6986a85e7950caefb717af910/src/types/utils.cpp#L701-L713
static PIPE_DIRECTORY: LazyLock<windows::core::Result<ShareableHandle>> =
    LazyLock::new(|| unsafe {
        let mut device_path = UNICODE_STRING::default();
        RtlInitUnicodeString(&mut device_path, windows::core::w!(r"\Device\NamedPipe\"));

        let mut object_attributes = new_object_attributes();
        object_attributes.ObjectName = &device_path;

        let mut io_status_block = IO_STATUS_BLOCK::default();
        let mut handle = HANDLE::default();
        windows::core::HRESULT::from(NtCreateFile(
            &mut handle,
            FILE_ACCESS_RIGHTS(GENERIC_READ.0 | SYNCHRONIZE.0),
            &object_attributes,
            &mut io_status_block,
            None,
            FILE_FLAGS_AND_ATTRIBUTES::default(),
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            FILE_OPEN,
            FILE_SYNCHRONOUS_IO_NONALERT,
            None,
            0,
        ))
        .ok()
        .map(|_| ShareableHandle(handle))
    });

/// The set of errors that can occur while creating a pipe.
#[derive(Debug, thiserror::Error)]
#[allow(clippy::enum_variant_names)]
pub enum CreatePipeError {
    #[error("Failed to open named pipe device: {0:#}")]
    PipeDeviceOpen(#[source] windows::core::Error),
    #[error("Failed to create pipe: {0:#}")]
    CreatePipe(#[source] windows::core::Error),
    #[error("Failed to create client-side handle: {0:#}")]
    ClientHandleCreation(#[source] windows::core::Error),
}

/// A bidirectional pipe.
pub struct DuplexPipe {
    /// The client-side (e.g.: OpenConsole) end of the pipe.
    pub client: HANDLE,
    /// The server-side (e.g.: Warp) end of the pipe.
    pub server: HANDLE,
}

/// Creates a bidirectional, asynchronous anonymous pipe.
///
/// This is based on the Windows Terminal code here:
/// https://github.com/microsoft/terminal/blob/93930bb3fa99d4e6986a85e7950caefb717af910/src/types/utils.cpp#L715-L774
pub fn create_async_anonymous_pipe() -> Result<DuplexPipe, CreatePipeError> {
    const BUFFER_SIZE: u32 = 128 * 1024;

    let empty_path = UNICODE_STRING::default();
    let mut object_attributes = new_object_attributes();
    object_attributes.ObjectName = &empty_path;
    object_attributes.Attributes = OBJ_CASE_INSENSITIVE;

    let desired_access = FILE_ACCESS_RIGHTS(GENERIC_READ.0 | GENERIC_WRITE.0 | SYNCHRONIZE.0);
    let share_access = FILE_SHARE_READ | FILE_SHARE_WRITE;

    let mut timeout = -1_000_000_000;
    let mut io_status_block = IO_STATUS_BLOCK::default();

    unsafe {
        let mut server = HANDLE::default();
        object_attributes.RootDirectory = PIPE_DIRECTORY
            .clone()
            .map_err(CreatePipeError::PipeDeviceOpen)?
            .0;
        windows::core::HRESULT::from(ffi::NtCreateNamedPipeFile(
            &mut server,
            desired_access.0,
            &mut object_attributes,
            &mut io_status_block,
            share_access.0,
            FILE_CREATE.0,
            // Synchronous pipes would set FILE_SYNCHRONOUS_IO_NONALERT here.
            NTCREATEFILE_CREATE_OPTIONS::default().0,
            FILE_PIPE_BYTE_STREAM_TYPE,
            FILE_PIPE_BYTE_STREAM_MODE,
            FILE_PIPE_QUEUE_OPERATION,
            1,
            BUFFER_SIZE,
            BUFFER_SIZE,
            &mut timeout,
        ))
        .ok()
        .map_err(CreatePipeError::CreatePipe)?;

        let mut client = HANDLE::default();
        object_attributes.RootDirectory = server;
        windows::core::HRESULT::from(NtCreateFile(
            &mut client,
            desired_access,
            &object_attributes,
            &mut io_status_block,
            None,
            FILE_FLAGS_AND_ATTRIBUTES::default(),
            share_access,
            FILE_OPEN,
            FILE_NON_DIRECTORY_FILE,
            None,
            0,
        ))
        .ok()
        .map_err(CreatePipeError::ClientHandleCreation)?;

        Ok(DuplexPipe { client, server })
    }
}

/// Constructs a new, empty [`OBJECT_ATTRIBUTES`].
fn new_object_attributes() -> OBJECT_ATTRIBUTES {
    OBJECT_ATTRIBUTES {
        Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
        ..Default::default()
    }
}

mod ffi {
    use super::*;

    #[link(name = "ntdll.dll", kind = "raw-dylib", modifiers = "+verbatim")]
    extern "system" {
        pub fn NtCreateNamedPipeFile(
            FileHandle: *mut HANDLE,
            DesiredAccess: u32,
            ObjectAttributes: *mut OBJECT_ATTRIBUTES,
            IoStatusBlock: *mut IO_STATUS_BLOCK,
            ShareAccess: u32,
            CreateDisposition: u32,
            CreateOptions: u32,
            NamedPipeType: u32,
            ReadMode: u32,
            CompletionMode: u32,
            MaximumInstances: u32,
            InboundQuota: u32,
            OutboundQuota: u32,
            DefaultTimeout: *mut i64,
        ) -> NTSTATUS;
    }
}
