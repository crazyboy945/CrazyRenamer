use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};

use eframe::egui;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DispatchMessageW,
    GetCursorPos, GetMessageW, LoadIconW, PostMessageW, RegisterClassW, SetForegroundWindow,
    TrackPopupMenu, TranslateMessage, IDI_APPLICATION, MF_SEPARATOR, MF_STRING, MSG,
    TPM_BOTTOMALIGN, TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_COMMAND,
    WM_CONTEXTMENU, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSW,
};

pub const STATUS_STARTING: u8 = 0;
pub const STATUS_RUNNING: u8 = 1;
pub const STATUS_FAILED: u8 = 2;

pub enum TrayCommand {
    Show,
    Exit,
}

pub struct TrayHandle {
    pub receiver: Receiver<TrayCommand>,
    status: Arc<AtomicU8>,
    error: Arc<Mutex<Option<String>>>,
}

impl TrayHandle {
    pub fn is_ready(&self) -> bool {
        self.status.load(Ordering::SeqCst) == STATUS_RUNNING
    }

    pub fn status(&self) -> u8 {
        self.status.load(Ordering::SeqCst)
    }

    pub fn error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|error| error.clone())
    }
}

struct TrayContext {
    ctx: egui::Context,
    sender: Mutex<Sender<TrayCommand>>,
}

static TRAY_CONTEXT: OnceLock<TrayContext> = OnceLock::new();

const TRAY_UID: u32 = 1;
const WM_TRAYICON: u32 = WM_APP + 1;
const CMD_SHOW: usize = 1;
const CMD_EXIT: usize = 2;

pub fn start(ctx: egui::Context) -> TrayHandle {
    let (sender, receiver) = channel();
    let status = Arc::new(AtomicU8::new(STATUS_STARTING));
    let error = Arc::new(Mutex::new(None));

    let _ = TRAY_CONTEXT.set(TrayContext {
        ctx: ctx.clone(),
        sender: Mutex::new(sender),
    });

    let thread_status = Arc::clone(&status);
    let thread_error = Arc::clone(&error);
    std::thread::Builder::new()
        .name("tray-icon".to_owned())
        .spawn(move || unsafe {
            run_tray(ctx, thread_status, thread_error);
        })
        .ok();

    TrayHandle {
        receiver,
        status,
        error,
    }
}

unsafe fn run_tray(
    ctx: egui::Context,
    status: Arc<AtomicU8>,
    error: Arc<Mutex<Option<String>>>,
) {
    if let Err(message) = unsafe { create_and_run(ctx.clone(), &status) } {
        *error.lock().unwrap() = Some(message);
        status.store(STATUS_FAILED, Ordering::SeqCst);
        ctx.request_repaint();
    }
}

unsafe fn create_and_run(ctx: egui::Context, status: &AtomicU8) -> Result<(), String> {
    let module =
        unsafe { GetModuleHandleW(None) }.map_err(|error| format!("无法获取程序模块：{error}"))?;
    let hinstance = HINSTANCE(module.0);
    let class_name = w!("CrazyRenamerTrayWindow");
    let window_class = WNDCLASSW {
        lpfnWndProc: Some(tray_wndproc),
        hInstance: hinstance,
        lpszClassName: class_name,
        ..Default::default()
    };
    unsafe { RegisterClassW(&window_class) };

    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            w!("CrazyRenamerTray"),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinstance),
            None,
        )
    }
    .map_err(|error| format!("无法创建托盘窗口：{error}"))?;

    let icon = unsafe { LoadIconW(Some(hinstance), PCWSTR(1usize as *const u16)) }
        .or_else(|_| unsafe { LoadIconW(None, IDI_APPLICATION) })
        .map_err(|error| format!("无法加载托盘图标：{error}"))?;

    let mut data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_UID,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: WM_TRAYICON,
        hIcon: icon,
        ..Default::default()
    };
    let tip: Vec<u16> = "CrazyRenamer - 鼠标中键批量更名"
        .encode_utf16()
        .take(127)
        .collect();
    data.szTip[..tip.len()].copy_from_slice(&tip);

    if !unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool() {
        return Err("系统托盘拒绝创建图标。".to_owned());
    }

    status.store(STATUS_RUNNING, Ordering::SeqCst);
    ctx.request_repaint();

    let mut message = MSG::default();
    loop {
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) };
        if result.0 <= 0 {
            break;
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
    Ok(())
}

fn send(command: TrayCommand) {
    let Some(context) = TRAY_CONTEXT.get() else {
        return;
    };
    if let Ok(sender) = context.sender.lock() {
        let _ = sender.send(command);
    }
    context.ctx.request_repaint();
}

unsafe extern "system" fn tray_wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_TRAYICON {
        match (lparam.0 as u32) & 0xFFFF {
            WM_LBUTTONUP | WM_LBUTTONDBLCLK => send(TrayCommand::Show),
            WM_RBUTTONUP | WM_CONTEXTMENU => unsafe { show_menu(hwnd) },
            _ => {}
        }
        return LRESULT(0);
    }

    if message == WM_COMMAND {
        match wparam.0 & 0xFFFF {
            CMD_SHOW => send(TrayCommand::Show),
            CMD_EXIT => send(TrayCommand::Exit),
            _ => {}
        }
        return LRESULT(0);
    }

    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

unsafe fn show_menu(hwnd: HWND) {
    let Ok(menu) = (unsafe { CreatePopupMenu() }) else {
        return;
    };
    let show: Vec<u16> = "显示主窗口\0".encode_utf16().collect();
    let exit: Vec<u16> = "退出 CrazyRenamer\0".encode_utf16().collect();
    let _ = unsafe { AppendMenuW(menu, MF_STRING, CMD_SHOW, PCWSTR(show.as_ptr())) };
    let _ = unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()) };
    let _ = unsafe { AppendMenuW(menu, MF_STRING, CMD_EXIT, PCWSTR(exit.as_ptr())) };

    let mut point = POINT::default();
    let _ = unsafe { GetCursorPos(&mut point) };
    let _ = unsafe { SetForegroundWindow(hwnd) };
    let _ = unsafe {
        TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            point.x,
            point.y,
            Some(0),
            hwnd,
            None,
        )
    };
    let _ = unsafe { DestroyMenu(menu) };
    let _ = unsafe { PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0)) };
}
