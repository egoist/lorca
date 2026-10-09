//! Windows' pseudo console (ConPTY), where `bash` runs a session's command as Windows Terminal
//! runs a shell: in a console of its own, which the command asks on and the session types into,
//! and which outlives the call. The shell starts in a job object, attached as it is created, so
//! stopping the session ends every process the command started.
//!
//! ConPTY came with Windows 10 1809. Its functions are looked up when first needed, and on a
//! Windows without them commands run on pipes ([`supported`]).

use std::ffi::{c_void, OsStr};
use std::fs::File;
use std::io::{Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE, S_OK};
use windows_sys::Win32::System::Console::{COORD, HPCON};
use windows_sys::Win32::System::JobObjects::{CreateJobObjectW, TerminateJobObject};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess, InitializeProcThreadAttributeList, UpdateProcThreadAttribute, WaitForSingleObject,
    CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, INFINITE, LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION,
    PROC_THREAD_ATTRIBUTE_JOB_LIST, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

type CreatePseudoConsole = unsafe extern "system" fn(COORD, HANDLE, HANDLE, u32, *mut HPCON) -> i32;
type ClosePseudoConsole = unsafe extern "system" fn(HPCON);
type ReleasePseudoConsole = unsafe extern "system" fn(HPCON) -> i32;

struct Functions {
    create: CreatePseudoConsole,
    close: ClosePseudoConsole,
    /// Windows 11 24H2 and later: the console's output then ends once the last process in it
    /// exits, as a terminal's does, instead of when the session closes it.
    release: Option<ReleasePseudoConsole>,
}

static FUNCTIONS: LazyLock<Option<Functions>> = LazyLock::new(|| unsafe {
    let kernel32 = GetModuleHandleW(wide(OsStr::new("kernel32.dll")).as_ptr());
    if kernel32.is_null() {
        return None;
    }
    let find = |name: &std::ffi::CStr| GetProcAddress(kernel32, name.as_ptr().cast());
    let create = find(c"CreatePseudoConsole")?;
    let close = find(c"ClosePseudoConsole")?;
    Some(Functions {
        create: std::mem::transmute::<unsafe extern "system" fn() -> isize, CreatePseudoConsole>(create),
        close: std::mem::transmute::<unsafe extern "system" fn() -> isize, ClosePseudoConsole>(close),
        release: find(c"ReleasePseudoConsole").map(|release| std::mem::transmute::<unsafe extern "system" fn() -> isize, ReleasePseudoConsole>(release)),
    })
});

/// Whether this Windows has ConPTY.
pub fn supported() -> bool {
    FUNCTIONS.is_some()
}

/// A command started in a console of its own.
pub struct Spawned {
    pub pid: u32,
    pub process: OwnedHandle,
    pub job: Job,
    pub console: Console,
    /// What the console shows, as the VT sequences a terminal draws. It ends once the console
    /// closes.
    pub output: File,
}

/// Starts `command` (its program, arguments, working directory, and environment changes) in a
/// new console of `columns` by `rows`, inside a new job.
pub fn spawn(command: &std::process::Command, columns: u16, rows: u16) -> std::io::Result<Spawned> {
    let functions = FUNCTIONS.as_ref().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Unsupported, "this Windows has no pseudo console"))?;
    let (input_read, input_write) = pipe()?;
    let (output_read, output_write) = pipe()?;
    let mut handle: HPCON = 0;
    let size = COORD { X: columns as i16, Y: rows as i16 };
    let result = unsafe { (functions.create)(size, input_read.as_raw_handle(), output_write.as_raw_handle(), 0, &mut handle) };
    if result != S_OK {
        return Err(std::io::Error::other(format!("Could not open a console (HRESULT {result:#x})")));
    }
    let console = Console { handle, input: Mutex::new(Some(File::from(input_write))), closed: AtomicBool::new(false) };
    // On a failure from here on, the console closes with nobody reading its output: let it go
    // first, so the close does not wait for a reader.
    let fail = |error: std::io::Error, output_read: OwnedHandle| {
        drop(output_read);
        error
    };
    let job = match Job::new() {
        Ok(job) => job,
        Err(error) => return Err(fail(error, output_read)),
    };
    let process = match create_process(command, handle, &job) {
        Ok(process) => process,
        Err(error) => return Err(fail(error, output_read)),
    };
    // The console keeps its own copies of its ends of the pipes. Ours would keep the output
    // open after the console closes it.
    drop(input_read);
    drop(output_write);
    if let Some(release) = functions.release {
        unsafe { release(handle) };
    }
    let pid = unsafe { windows_sys::Win32::System::Threading::GetProcessId(process.as_raw_handle()) };
    Ok(Spawned { pid, process, job, console, output: File::from(output_read) })
}

/// Waits for the process to exit and returns its exit code. Blocks.
pub fn wait(process: &OwnedHandle) -> std::io::Result<u32> {
    unsafe { WaitForSingleObject(process.as_raw_handle(), INFINITE) };
    let mut code = 0u32;
    if unsafe { GetExitCodeProcess(process.as_raw_handle(), &mut code) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(code)
}

/// Reads what the console shows, `chunk` by chunk, until the console closes. Blocks.
pub fn read_until_closed(mut output: File, mut chunk: impl FnMut(&[u8])) {
    let mut buffer = vec![0u8; 16 * 1024];
    while let Ok(n) = output.read(&mut buffer) {
        if n == 0 {
            break;
        }
        chunk(&buffer[..n]);
    }
}

/// The console a command runs in: its input, which the session types into, until it closes.
pub struct Console {
    handle: HPCON,
    input: Mutex<Option<File>>,
    closed: AtomicBool,
}

impl Console {
    /// Types `keys`, as `bash_session::console_keys` made them. Blocks while the console takes them.
    pub fn write(&self, keys: &[u8]) -> std::io::Result<()> {
        let mut input = self.input.lock().unwrap();
        let input = input.as_mut().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "The command has ended"))?;
        input.write_all(keys)
    }

    /// Closes the console, which ends its output once what it holds is read. On Windows before
    /// 11 24H2 this also ends the processes still in it. Closing waits for the output to be
    /// read, so it happens on a thread of its own while the reader reads on.
    pub fn close(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        self.input.lock().unwrap().take();
        let Some(functions) = FUNCTIONS.as_ref() else { return };
        let (close, handle) = (functions.close, self.handle);
        std::thread::spawn(move || unsafe { close(handle) });
    }
}

impl Drop for Console {
    fn drop(&mut self) {
        self.close();
    }
}

/// The job a command's processes run in. Terminating it ends them all, also one whose parent
/// has exited. Breakaway stays off: the Cygwin runtime under Git Bash moves every process it
/// starts out of a job that allows it.
pub struct Job(OwnedHandle);

impl Job {
    fn new() -> std::io::Result<Job> {
        // No limits: the processes outlive the job's handle, as a command left running does.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Job(unsafe { OwnedHandle::from_raw_handle(handle) }))
    }

    /// Ends every process in the job.
    pub fn terminate(&self) {
        unsafe { TerminateJobObject(self.0.as_raw_handle(), 1) };
    }
}

/// An anonymous pipe, synchronous, as ConPTY takes them: (read end, write end).
fn pipe() -> std::io::Result<(OwnedHandle, OwnedHandle)> {
    let (mut read, mut write): (HANDLE, HANDLE) = (std::ptr::null_mut(), std::ptr::null_mut());
    if unsafe { CreatePipe(&mut read, &mut write, std::ptr::null(), 0) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { (OwnedHandle::from_raw_handle(read), OwnedHandle::from_raw_handle(write)) })
}

/// Creates the process in `console` and `job`, both attached as it is created, so nothing it
/// starts runs outside them.
fn create_process(command: &std::process::Command, console: HPCON, job: &Job) -> std::io::Result<OwnedHandle> {
    let program = command.get_program();
    // As the standard library writes it for `Command`, so a command reaches Git Bash as it does
    // on pipes: the program in quotes, each argument after it as `append_arg` quotes it.
    let mut line = vec![b'"' as u16];
    line.extend(program.encode_wide());
    line.push(b'"' as u16);
    for arg in command.get_args() {
        line.push(b' ' as u16);
        crate::login_shell::append_arg(&mut line, arg.encode_wide());
    }
    if line.contains(&0) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "A command cannot hold a NUL character"));
    }
    line.push(0);
    let application = std::path::Path::new(program).is_absolute().then(|| wide(program));
    let environment = environment_block(command.get_envs());
    let directory = command.get_current_dir().map(|dir| wide(dir.as_os_str()));

    let mut attributes = AttributeList::new(2)?;
    // The pseudo console's attribute is the handle itself; the job list's is a pointer to the
    // handles, which must outlive the call.
    attributes.update(PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, console as *const c_void, std::mem::size_of::<HPCON>())?;
    let jobs = [job.0.as_raw_handle()];
    attributes.update(PROC_THREAD_ATTRIBUTE_JOB_LIST, jobs.as_ptr().cast(), std::mem::size_of_val(&jobs))?;

    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    // Standard handles of its own, none: else a CLI whose own output is a pipe passes it on,
    // and the command writes there instead of to its console.
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
    startup.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
    startup.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
    startup.lpAttributeList = attributes.as_mut_ptr();
    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let created = unsafe {
        CreateProcessW(
            application.as_ref().map_or(std::ptr::null(), |name| name.as_ptr()),
            line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            environment.as_ptr().cast(),
            directory.as_ref().map_or(std::ptr::null(), |dir| dir.as_ptr()),
            &startup.StartupInfo,
            &mut info,
        )
    };
    if created == 0 {
        let error = std::io::Error::last_os_error();
        return Err(std::io::Error::new(error.kind(), format!("Failed to start {}: {error}", program.to_string_lossy())));
    }
    drop(unsafe { OwnedHandle::from_raw_handle(info.hThread) });
    Ok(unsafe { OwnedHandle::from_raw_handle(info.hProcess) })
}

/// A process-thread attribute list, in memory aligned for the pointers it holds.
struct AttributeList(Vec<usize>);

impl AttributeList {
    fn new(count: u32) -> std::io::Result<AttributeList> {
        let mut size = 0usize;
        unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), count, 0, &mut size) };
        let mut list = AttributeList(vec![0usize; size.div_ceil(std::mem::size_of::<usize>())]);
        if unsafe { InitializeProcThreadAttributeList(list.as_mut_ptr(), count, 0, &mut size) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(list)
    }

    fn as_mut_ptr(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.0.as_mut_ptr().cast()
    }

    fn update(&mut self, attribute: u32, value: *const c_void, size: usize) -> std::io::Result<()> {
        if unsafe { UpdateProcThreadAttribute(self.as_mut_ptr(), 0, attribute as usize, value, size, std::ptr::null_mut(), std::ptr::null()) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        unsafe { DeleteProcThreadAttributeList(self.as_mut_ptr()) };
    }
}

/// This process's environment with `changes` over it (a value sets a variable, none removes
/// it), names compared without case as Windows does, sorted as `CreateProcessW` wants them.
fn environment_block<'a>(changes: impl Iterator<Item = (&'a OsStr, Option<&'a OsStr>)>) -> Vec<u16> {
    let key = |name: &OsStr| name.to_string_lossy().to_uppercase();
    let mut variables: std::collections::BTreeMap<String, (std::ffi::OsString, std::ffi::OsString)> =
        std::env::vars_os().map(|(name, value)| (key(&name), (name, value))).collect();
    for (name, value) in changes {
        match value {
            Some(value) => variables.insert(key(name), (name.to_owned(), value.to_owned())),
            None => variables.remove(&key(name)),
        };
    }
    let mut block = Vec::new();
    for (name, value) in variables.values() {
        block.extend(name.encode_wide());
        block.push(b'=' as u16);
        block.extend(value.encode_wide());
        block.push(0);
    }
    block.push(0);
    block
}

fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(std::iter::once(0)).collect()
}
