//! Windows builds are GUI-subsystem programs and start without a console. The
//! wizard needs one, so `--setup` borrows the parent's (the shell that ran
//! it) or opens a new window when there is none.

/// Gets a console for the wizard. True when a new window was opened for it:
/// it closes with the process, so the caller should wait before exiting.
#[cfg(windows)]
pub fn attach() -> bool {
    use std::fs::OpenOptions;
    use std::os::windows::io::IntoRawHandle;

    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS,
        AllocConsole,
        AttachConsole,
        GetStdHandle,
        STD_ERROR_HANDLE,
        STD_INPUT_HANDLE,
        STD_OUTPUT_HANDLE,
        SetStdHandle,
    };

    // SAFETY: both calls take no pointers and only change this process's
    // console attachment.
    let opened_new = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) == 0 && AllocConsole() != 0 };

    // A GUI process has no standard handles, and attaching does not create
    // them. Point the ones that are missing at the console, so `println!` and
    // `is_terminal` work; handles redirected to a file or pipe stay as they
    // are.
    for (id, device, write) in [
        (STD_INPUT_HANDLE, "CONIN$", false),
        (STD_OUTPUT_HANDLE, "CONOUT$", true),
        (STD_ERROR_HANDLE, "CONOUT$", true),
    ] {
        // SAFETY: `GetStdHandle` only reads the process's handle table.
        let current = unsafe { GetStdHandle(id) };
        if !current.is_null() && current != INVALID_HANDLE_VALUE {
            continue;
        }
        if let Ok(file) = OpenOptions::new()
            .read(!write)
            .write(write)
            .open(device)
        {
            // SAFETY: the handle is valid and deliberately never closed: it
            // must outlive every later use of the standard stream.
            unsafe { SetStdHandle(id, file.into_raw_handle()) };
        }
    }
    opened_new
}

#[cfg(not(windows))]
pub const fn attach() -> bool {
    false
}
