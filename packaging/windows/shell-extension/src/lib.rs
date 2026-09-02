#![cfg(windows)]
#![allow(non_snake_case)]

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use windows::core::{implement, Error, Interface, GUID, HRESULT, PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    BOOL, CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_NOTIMPL, E_POINTER, HMODULE, S_FALSE,
    S_OK,
};
use windows::Win32::System::Com::{
    CoTaskMemAlloc, CoTaskMemFree, IBindCtx, IClassFactory, IClassFactory_Impl,
};
use windows::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};
use windows::Win32::UI::Shell::{
    IEnumExplorerCommand, IExplorerCommand, IExplorerCommand_Impl, IShellItemArray, ShellExecuteW,
    SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

pub const COMMAND_CLSID: GUID =
    GUID::from_u128(email_to_markdown_shell_extension_contract::COMMAND_CLSID_U128);
static OBJECT_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Increments `OBJECT_COUNT` on construction and decrements it on `Drop`.
/// Shared by every COM object that must keep the DLL alive while it exists,
/// so `DllCanUnloadNow` reflects the true outstanding-instance count.
struct RefCountGuard;

impl RefCountGuard {
    fn new() -> Self {
        OBJECT_COUNT.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for RefCountGuard {
    fn drop(&mut self) {
        OBJECT_COUNT.fetch_sub(1, Ordering::SeqCst);
    }
}

fn quote_command_line_argument(value: &str) -> String {
    if !value
        .chars()
        .any(|character| character == ' ' || character == '\t' || character == '"')
    {
        return value.to_owned();
    }

    let mut quoted = String::from("\"");
    let mut backslashes = 0;
    for character in value.chars() {
        if character == '\\' {
            backslashes += 1;
            continue;
        }
        if character == '"' {
            quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
            quoted.push('"');
        } else {
            quoted.push_str(&"\\".repeat(backslashes));
            quoted.push(character);
        }
        backslashes = 0;
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn task_allocated_wide(value: &str) -> windows::core::Result<PWSTR> {
    let encoded = wide(value);
    let bytes = encoded.len() * std::mem::size_of::<u16>();
    let destination = unsafe { CoTaskMemAlloc(bytes) } as *mut u16;
    if destination.is_null() {
        return Err(Error::from_hresult(
            windows::Win32::Foundation::E_OUTOFMEMORY,
        ));
    }
    unsafe { destination.copy_from_nonoverlapping(encoded.as_ptr(), encoded.len()) };
    Ok(PWSTR(destination))
}

fn module_path() -> windows::core::Result<PathBuf> {
    let mut module = HMODULE::default();
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(DllGetClassObject as *const () as *const u16),
            &mut module,
        )?;
    }
    let mut buffer = vec![0u16; 32_768];
    let length = unsafe { GetModuleFileNameW(module, &mut buffer) } as usize;
    if length == 0 || length >= buffer.len() {
        return Err(Error::from_win32());
    }
    Ok(PathBuf::from(String::from_utf16_lossy(&buffer[..length])))
}

/// "email-to-markdown.exe" and "email-to-markdown-mail.ico" below are manually
/// kept in sync with the main crate's binary name and `MAIL_ICON_NAME`
/// constant. No shared crate: two stable literals do not justify the coupling.
fn executable_path() -> windows::core::Result<PathBuf> {
    Ok(module_path()?
        .parent()
        .ok_or_else(|| Error::from(E_POINTER))?
        .join("email-to-markdown.exe"))
}

fn icon_path() -> windows::core::Result<PathBuf> {
    let icon = module_path()?
        .parent()
        .ok_or_else(|| Error::from(E_POINTER))?
        .join("email-to-markdown-mail.ico");
    if icon.is_file() {
        Ok(icon)
    } else {
        executable_path()
    }
}

fn launch_for_path(path: &str) -> windows::core::Result<()> {
    let executable = executable_path()?;
    let parameters = format!("contextual {}", quote_command_line_argument(path));
    let executable_wide = wide(&executable.to_string_lossy());
    let parameters_wide = wide(&parameters);
    let working_directory = executable.parent().ok_or_else(|| Error::from(E_POINTER))?;
    let working_directory_wide = wide(&working_directory.to_string_lossy());
    let result = unsafe {
        ShellExecuteW(
            None,
            None,
            PCWSTR(executable_wide.as_ptr()),
            PCWSTR(parameters_wide.as_ptr()),
            PCWSTR(working_directory_wide.as_ptr()),
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize <= 32 {
        return Err(Error::from_win32());
    }
    Ok(())
}

#[implement(IExplorerCommand)]
struct ExplorerCommand {
    _ref_count: RefCountGuard,
}

impl ExplorerCommand {
    fn new() -> Self {
        Self {
            _ref_count: RefCountGuard::new(),
        }
    }
}

impl IExplorerCommand_Impl for ExplorerCommand {
    fn GetTitle(&self, _items: Option<&IShellItemArray>) -> windows::core::Result<PWSTR> {
        task_allocated_wide("Importer les emails en Markdown")
    }

    fn GetIcon(&self, _items: Option<&IShellItemArray>) -> windows::core::Result<PWSTR> {
        task_allocated_wide(&icon_path()?.to_string_lossy())
    }

    fn GetToolTip(&self, _items: Option<&IShellItemArray>) -> windows::core::Result<PWSTR> {
        Err(Error::from(E_NOTIMPL))
    }

    fn GetCanonicalName(&self) -> windows::core::Result<GUID> {
        Ok(COMMAND_CLSID)
    }

    fn GetState(
        &self,
        _items: Option<&IShellItemArray>,
        _ok_to_be_slow: BOOL,
    ) -> windows::core::Result<u32> {
        Ok(0)
    }

    fn Invoke(
        &self,
        items: Option<&IShellItemArray>,
        _bind_context: Option<&IBindCtx>,
    ) -> windows::core::Result<()> {
        let items = items.ok_or_else(|| Error::from(E_POINTER))?;
        let count = unsafe { items.GetCount()? };
        if count == 0 {
            return Err(Error::from(E_POINTER));
        }
        let item = unsafe { items.GetItemAt(0)? };
        let raw_path = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH)? };
        let path = unsafe { raw_path.to_string() };
        unsafe { CoTaskMemFree(Some(raw_path.0.cast())) };
        launch_for_path(&path?)
    }

    fn GetFlags(&self) -> windows::core::Result<u32> {
        Ok(0)
    }

    fn EnumSubCommands(&self) -> windows::core::Result<IEnumExplorerCommand> {
        Err(Error::from(E_NOTIMPL))
    }
}

#[implement(IClassFactory)]
struct CommandClassFactory {
    _ref_count: RefCountGuard,
}

impl CommandClassFactory {
    fn new() -> Self {
        Self {
            _ref_count: RefCountGuard::new(),
        }
    }
}

impl IClassFactory_Impl for CommandClassFactory {
    fn CreateInstance(
        &self,
        outer: Option<&windows::core::IUnknown>,
        interface_id: *const GUID,
        object: *mut *mut c_void,
    ) -> windows::core::Result<()> {
        if object.is_null() || interface_id.is_null() {
            return Err(Error::from(E_POINTER));
        }
        unsafe { *object = std::ptr::null_mut() };
        if outer.is_some() {
            return Err(Error::from(CLASS_E_NOAGGREGATION));
        }
        let command: IExplorerCommand = ExplorerCommand::new().into();
        let status = unsafe { command.query(interface_id, object) };
        status.ok()
    }

    fn LockServer(&self, lock: BOOL) -> windows::core::Result<()> {
        if lock.as_bool() {
            OBJECT_COUNT.fetch_add(1, Ordering::SeqCst);
        } else {
            OBJECT_COUNT.fetch_sub(1, Ordering::SeqCst);
        }
        Ok(())
    }
}

#[no_mangle]
pub unsafe extern "system" fn DllGetClassObject(
    class_id: *const GUID,
    interface_id: *const GUID,
    object: *mut *mut c_void,
) -> HRESULT {
    if class_id.is_null() || interface_id.is_null() || object.is_null() {
        return E_POINTER;
    }
    *object = std::ptr::null_mut();
    if *class_id != COMMAND_CLSID {
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    let factory: IClassFactory = CommandClassFactory::new().into();
    factory.query(interface_id, object)
}

#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if OBJECT_COUNT.load(Ordering::SeqCst) == 0 {
        S_OK
    } else {
        S_FALSE
    }
}

#[cfg(test)]
mod tests {
    use super::quote_command_line_argument;

    #[test]
    fn quotes_paths_for_command_line_to_argv_w() {
        assert_eq!(
            quote_command_line_argument(r"C:\Notes\Client Simple"),
            r#""C:\Notes\Client Simple""#
        );
        assert_eq!(
            quote_command_line_argument(r"C:\Notes\Client"),
            r"C:\Notes\Client"
        );
        assert_eq!(
            quote_command_line_argument(r#"C:\A "B""#),
            r#""C:\A \"B\"""#
        );
    }
}
