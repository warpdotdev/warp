use std::ffi::{OsStr, c_void};
use std::io;
use std::os::windows::process::CommandExt as _;

use anyhow::{Context, Result};
use warp_errors::report_error;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
    QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};
use windows::core::PCWSTR;

#[derive(Debug, thiserror::Error)]
pub enum JobObjectError {
    #[error("Failed to create job: {0}")]
    CreateFailed(std::io::Error),

    #[error("Failed to assign process to job: {0}")]
    AssignFailed(std::io::Error),

    #[error("Failed to set info for job: {0}")]
    SetInfoFailed(std::io::Error),

    #[error("Failed to get info for job: {0}")]
    GetInfoFailed(std::io::Error),

    #[error(transparent)]
    Other(anyhow::Error),
}

impl From<win32job::JobError> for JobObjectError {
    fn from(error: win32job::JobError) -> Self {
        match error {
            win32job::JobError::CreateFailed(e) => JobObjectError::CreateFailed(e),
            win32job::JobError::AssignFailed(e) => JobObjectError::AssignFailed(e),
            win32job::JobError::SetInfoFailed(e) => JobObjectError::SetInfoFailed(e),
            win32job::JobError::GetInfoFailed(e) => JobObjectError::GetInfoFailed(e),
            _ => JobObjectError::Other(error.into()),
        }
    }
}

/// We use Job Objects to handle killing child processes when the program is
/// closed. This builder struct is used to configure a Job Object and associate it
/// with processes. Processes associated with a job will be killed when the handle
/// to the job is dropped at the end of the program's lifecycle.
///
/// NOTE: We've encountered issues with assigning some processes to jobs that
/// already contain other processes (i.e. `pwsh.exe`), so we only want to
/// assign a single process to a job.
///
/// For more information on Job Objects, see:
/// https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects
#[derive(Debug, Default)]
pub struct JobObject {
    assign_current_process: bool,
    assign_process: Option<isize>,
    kill_children_on_close: bool,
}

impl JobObject {
    pub fn new() -> Self {
        Self::default()
    }

    /// Assigns the current process to the Job Object. This can be used to ensure
    /// that children of the current process are associated with the job.
    pub fn assign_current_process(mut self) -> Self {
        self.assign_current_process = true;
        self
    }

    /// Assigns a process to the Job Object. This process will be killed when the
    /// current process is closed.
    pub fn assign_process(mut self, process: isize) -> Self {
        self.assign_process = Some(process);
        self
    }

    /// Configures the Job Object so children of the assigned processes are
    /// automatically associated with the job, thus killing them along with
    /// their parents on close.
    pub fn kill_children_on_close(mut self) -> Self {
        self.kill_children_on_close = true;
        self
    }

    fn create_internal(self) -> Result<(), win32job::JobError> {
        let job = win32job::Job::create()?;

        let mut info = job.query_extended_limit_info()?;
        // Mark the job as "kill on job close", so all processes associated with
        // the job are killed when the handle to the job is closed.
        info.limit_kill_on_job_close();
        info.limit_breakaway_ok();
        if !self.kill_children_on_close {
            info.limit_silent_breakaway_ok();
        }
        job.set_extended_limit_info(&info)?;

        if self.assign_current_process {
            job.assign_current_process()?;
        }
        if let Some(process) = self.assign_process {
            job.assign_process(process)?;
        }

        Box::leak(Box::new(job));
        Ok(())
    }

    /// Creates a new Job Object and assigns any specified processes to it. The
    /// handle to the job is leaked to ensure that the job lives for the lifetime
    /// of the program.
    pub fn create(self) -> Result<(), JobObjectError> {
        self.create_internal().map_err(Into::into)
    }
}

pub fn init() {
    if let Err(e) = JobObject::new()
        .kill_children_on_close()
        .assign_current_process()
        .create()
        .context("Failed to create job object for the program")
    {
        report_error!(e);
    }
}

/// Exit code the processes in a terminated job report; nonzero so an ended child is never mistaken
/// for a successful one.
const TERMINATED_EXIT_CODE: u32 = 1;

/// A Job Object that holds one spawned process and everything it starts. The job is created with
/// kill-on-close and no breakaway, so the whole tree ends when the job is terminated or when the
/// last handle to it closes, including when this process exits without running destructors.
///
/// Unlike [`JobObject`], the handle is owned and closed on drop, which is what a job scoped to one
/// short-lived child wants. It calls the `windows` crate directly because `win32job::Job` has no
/// way to terminate a job.
#[derive(Debug)]
pub struct ProcessJob {
    // Kept as an integer so the job can travel with a `Send` future; `HANDLE` wraps a raw pointer.
    handle: isize,
}

impl ProcessJob {
    /// Creates a job and assigns the process with id `pid` to it. The caller must still hold the
    /// process alive (for example through its `Child`) so the id cannot be recycled before the
    /// assignment.
    pub fn create_for(pid: u32) -> io::Result<Self> {
        // SAFETY: plain Win32 calls with valid arguments; every handle opened here is closed on
        // every path (the job through `Drop`, the process handle right after assignment).
        unsafe {
            let job = Self {
                handle: CreateJobObjectW(None, PCWSTR::null())?.0 as isize,
            };
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job.handle(),
                JobObjectExtendedLimitInformation,
                &limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION as *const c_void,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )?;
            let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, pid)?;
            let assigned = AssignProcessToJobObject(job.handle(), process);
            let _ = CloseHandle(process);
            assigned?;
            Ok(job)
        }
    }

    /// Ends every process in the job now.
    pub fn terminate(&self) -> io::Result<()> {
        // SAFETY: the handle is a live job handle owned by `self`.
        unsafe { TerminateJobObject(self.handle(), TERMINATED_EXIT_CODE) }.map_err(Into::into)
    }

    /// How many processes in the job are still running.
    pub fn active_processes(&self) -> io::Result<u32> {
        let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        // SAFETY: `accounting` is a correctly sized, writable buffer for the requested class.
        unsafe {
            QueryInformationJobObject(
                Some(self.handle()),
                JobObjectBasicAccountingInformation,
                &mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION as *mut c_void,
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                None,
            )?;
        }
        Ok(accounting.ActiveProcesses)
    }

    fn handle(&self) -> HANDLE {
        HANDLE(self.handle as *mut c_void)
    }
}

impl Drop for ProcessJob {
    fn drop(&mut self) {
        // SAFETY: the handle is owned by `self` and closed exactly once.
        let _ = unsafe { CloseHandle(self.handle()) };
    }
}

pub trait CommandExt {
    /// Append literal text to the command line without any quoting or escaping.
    ///
    /// This is useful for passing arguments to `cmd.exe /c`, which doesn't follow
    /// `CommandLineToArgvW` escaping rules.
    fn raw_arg<S: AsRef<OsStr>>(&mut self, text_to_append_as_is: S) -> &mut Self;
}

use async_process::windows::CommandExt as _;

impl CommandExt for crate::blocking::Command {
    fn raw_arg<S: AsRef<OsStr>>(&mut self, text_to_append_as_is: S) -> &mut Self {
        self.inner.raw_arg(text_to_append_as_is);
        self
    }
}

impl CommandExt for crate::r#async::Command {
    fn raw_arg<S: AsRef<OsStr>>(&mut self, text_to_append_as_is: S) -> &mut Self {
        self.inner.raw_arg(text_to_append_as_is);
        self
    }
}
