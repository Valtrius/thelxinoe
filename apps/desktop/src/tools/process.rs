use crate::error::{AppError, AppResult};
use std::{
    mem::{size_of, zeroed},
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
};
use tokio::process::{Child, Command};
use windows_sys::Win32::{
    Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject,
        },
        Threading::{
            CREATE_NO_WINDOW, CREATE_SUSPENDED, OpenProcess, OpenThread, PROCESS_SET_QUOTA,
            PROCESS_TERMINATE, ResumeThread, THREAD_SUSPEND_RESUME,
        },
    },
};

/// Owns a process tree. The suspended launch is assigned before any child can run.
pub struct ProcessJob(OwnedHandle);
impl ProcessJob {
    pub fn spawn(command: &mut Command) -> AppResult<(Child, Self)> {
        let job = Self::new()?;
        command
            .creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED)
            .kill_on_drop(true);
        let mut child = command.spawn()?;
        let pid = child
            .id()
            .ok_or_else(|| AppError::internal("Missing child PID"))?;
        if let Err(error) = job.assign_and_resume(pid) {
            let _ = child.start_kill();
            return Err(error);
        }
        Ok((child, job))
    }
    fn new() -> AppResult<Self> {
        // SAFETY: valid zero-initialized Windows structures, owned kernel handle.
        unsafe {
            let raw = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if raw.is_null() {
                return Err(std::io::Error::last_os_error().into());
            }
            let handle = OwnedHandle::from_raw_handle(raw);
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                raw,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) == 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
            Ok(Self(handle))
        }
    }
    fn assign_and_resume(&self, pid: u32) -> AppResult<()> {
        // SAFETY: process is ours and suspended; temporary handles are closed on every path.
        unsafe {
            let raw = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
            if raw.is_null() {
                return Err(std::io::Error::last_os_error().into());
            }
            let process = OwnedHandle::from_raw_handle(raw);
            if AssignProcessToJobObject(self.0.as_raw_handle(), process.as_raw_handle()) == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return Err(std::io::Error::last_os_error().into());
            }
            let snapshot = OwnedHandle::from_raw_handle(snapshot);
            let mut entry: THREADENTRY32 = zeroed();
            entry.dwSize = size_of::<THREADENTRY32>() as u32;
            let mut more = Thread32First(snapshot.as_raw_handle(), &mut entry);
            while more != 0 {
                if entry.th32OwnerProcessID == pid {
                    let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                    if thread.is_null() {
                        return Err(std::io::Error::last_os_error().into());
                    }
                    let result = ResumeThread(thread);
                    CloseHandle(thread);
                    if result == u32::MAX {
                        return Err(std::io::Error::last_os_error().into());
                    }
                    return Ok(());
                }
                more = Thread32Next(snapshot.as_raw_handle(), &mut entry);
            }
        }
        Err(AppError::internal(
            "Could not resume the owned playback process",
        ))
    }
    pub fn terminate(&self) {
        // SAFETY: this is our live job handle; termination only affects assigned processes.
        unsafe {
            TerminateJobObject(self.0.as_raw_handle(), 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn active_processes(job: &ProcessJob) -> u32 {
        use windows_sys::Win32::System::JobObjects::{
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectBasicAccountingInformation,
            QueryInformationJobObject,
        };
        // SAFETY: the job is live and the output buffer matches the requested structure.
        unsafe {
            let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = zeroed();
            assert_ne!(
                QueryInformationJobObject(
                    job.0.as_raw_handle(),
                    JobObjectBasicAccountingInformation,
                    &mut info as *mut _ as *mut _,
                    size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                    std::ptr::null_mut()
                ),
                0
            );
            info.ActiveProcesses
        }
    }
    #[tokio::test]
    async fn owned_process_runs_and_job_can_stop_it() {
        let mut cmd = Command::new("cmd.exe");
        cmd.args(["/d", "/c", "ping -n 30 127.0.0.1 > nul"]);
        let (mut child, job) = ProcessJob::spawn(&mut cmd).unwrap();
        assert!(child.try_wait().unwrap().is_none());
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while active_processes(&job) < 2 {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the spawned ping must belong to the same job");
        job.terminate();
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(5), child.wait())
                .await
                .unwrap()
                .is_ok()
        );
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while active_processes(&job) != 0 {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("termination must stop child processes too");
    }
}
