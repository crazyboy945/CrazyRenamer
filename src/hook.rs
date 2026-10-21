use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};

use eframe::egui;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, CallNextHookEx, DispatchMessageW, FindWindowW, GetForegroundWindow,
    GetMessageW, GetWindowThreadProcessId, IsIconic, SetForegroundWindow, SetWindowsHookExW,
    ShowWindow, TranslateMessage, MSG, SW_RESTORE, WH_MOUSE_LL, WM_MBUTTONDBLCLK, WM_MBUTTONDOWN,
    WM_MBUTTONUP,
};

pub const STATUS_STARTING: u8 = 0;
pub const STATUS_RUNNING: u8 = 1;
pub const STATUS_FAILED: u8 = 2;

pub struct HookHandle {
    pub receiver: Receiver<isize>,
    pub enabled: Arc<AtomicBool>,
    pub status: Arc<AtomicU8>,
    pub error: Arc<Mutex<Option<String>>>,
}

struct HookContext {
    enabled: Arc<AtomicBool>,
    sender: Mutex<Sender<isize>>,
    ctx: egui::Context,
}

static HOOK_CONTEXT: OnceLock<HookContext> = OnceLock::new();

pub fn start(ctx: egui::Context) -> HookHandle {
    let (sender, receiver) = channel::<isize>();
    let enabled = Arc::new(AtomicBool::new(true));
    let status = Arc::new(AtomicU8::new(STATUS_STARTING));
    let error = Arc::new(Mutex::new(None));

    let _ = HOOK_CONTEXT.set(HookContext {
        enabled: Arc::clone(&enabled),
        sender: Mutex::new(sender),
        ctx: ctx.clone(),
    });

    let thread_status = Arc::clone(&status);
    let thread_error = Arc::clone(&error);
    std::thread::Builder::new()
        .name("mouse-hook".to_owned())
        .spawn(move || unsafe {
            let hinstance = match GetModuleHandleW(None) {
                Ok(module) => HINSTANCE(module.0),
                Err(err) => {
                    *thread_error.lock().unwrap() = Some(format!("无法获取程序模块：{err}"));
                    thread_status.store(STATUS_FAILED, Ordering::SeqCst);
                    ctx.request_repaint();
                    return;
                }
            };
            let hook = match SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), Some(hinstance), 0) {
                Ok(hook) => hook,
                Err(err) => {
                    *thread_error.lock().unwrap() = Some(format!("无法安装鼠标钩子：{err}"));
                    thread_status.store(STATUS_FAILED, Ordering::SeqCst);
                    ctx.request_repaint();
                    return;
                }
            };
            thread_status.store(STATUS_RUNNING, Ordering::SeqCst);
            ctx.request_repaint();

            let mut message = MSG::default();
            loop {
                let result = GetMessageW(&mut message, None, 0, 0);
                if result.0 <= 0 {
                    break;
                }
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            let _ = windows::Win32::UI::WindowsAndMessaging::UnhookWindowsHookEx(hook);
        })
        .ok();

    HookHandle {
        receiver,
        enabled,
        status,
        error,
    }
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }
    let message = wparam.0 as u32;
    if !matches!(message, WM_MBUTTONDOWN | WM_MBUTTONUP | WM_MBUTTONDBLCLK) {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }
    let Some(context) = HOOK_CONTEXT.get() else {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    };
    if !context.enabled.load(Ordering::Relaxed) {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }
    let foreground = unsafe { GetForegroundWindow() };
    if !crate::explorer::is_middle_click_inside(foreground) {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    if message == WM_MBUTTONUP {
        if let Ok(sender) = context.sender.lock() {
            let _ = sender.send(foreground.0 as isize);
        }
        context.ctx.request_repaint();
    }
    LRESULT(1)
}

pub fn focus_main_window(title: &str) {
    let wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    let Ok(hwnd) = (unsafe { FindWindowW(None, PCWSTR(wide.as_ptr())) }) else {
        return;
    };
    if hwnd.is_invalid() {
        return;
    }
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let foreground = GetForegroundWindow();
        if foreground == hwnd {
            return;
        }
        let foreground_thread = if foreground.is_invalid() {
            0
        } else {
            GetWindowThreadProcessId(foreground, None)
        };
        let current_thread = GetCurrentThreadId();
        let attached = foreground_thread != 0
            && foreground_thread != current_thread
            && AttachThreadInput(current_thread, foreground_thread, true).as_bool();
        let _ = SetForegroundWindow(hwnd);
        let _ = BringWindowToTop(hwnd);
        if attached {
            let _ = AttachThreadInput(current_thread, foreground_thread, false);
        }
    }
}
