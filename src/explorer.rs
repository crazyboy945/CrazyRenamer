use std::path::PathBuf;
use windows::core::{w, BSTR, Interface};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, IServiceProvider, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Variant::{VARIANT, VT_I4};
use windows::Win32::UI::Shell::{
    IShellBrowser, IShellWindows, IWebBrowser2, PathCreateFromUrlW, ShellWindows,
    SID_STopLevelBrowser,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GetAncestor, GetClassNameW, GetCursorPos, WindowFromPoint, GA_ROOT,
};

pub fn init_com() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
}

pub fn is_explorer_window(hwnd: HWND) -> bool {
    if hwnd.is_invalid() {
        return false;
    }
    let mut class_name = [0u16; 64];
    let length = unsafe { GetClassNameW(hwnd, &mut class_name) };
    if length <= 0 {
        return false;
    }
    matches!(
        String::from_utf16_lossy(&class_name[..length as usize]).as_str(),
        "CabinetWClass" | "ExploreWClass"
    )
}

pub fn is_middle_click_inside(hwnd: HWND) -> bool {
    if !is_explorer_window(hwnd) {
        return false;
    }
    let mut point = Default::default();
    if unsafe { GetCursorPos(&mut point) }.is_err() {
        return false;
    }
    let clicked = unsafe { WindowFromPoint(point) };
    if clicked.is_invalid() {
        return false;
    }
    unsafe { GetAncestor(clicked, GA_ROOT) == hwnd }
}

pub fn active_folder(foreground: HWND) -> Result<PathBuf, String> {
    if !is_explorer_window(foreground) {
        return Err("当前前台窗口不是资源管理器。".to_owned());
    }
    let active_tab = unsafe { FindWindowExW(Some(foreground), None, w!("ShellTabWindowClass"), None) }
        .ok()
        .filter(|hwnd| !hwnd.is_invalid());

    unsafe {
        let shell_windows: IShellWindows =
            CoCreateInstance(&ShellWindows, None, CLSCTX_ALL).map_err(|error| {
                format!("无法连接资源管理器（Shell COM）：{error}")
            })?;
        let count = shell_windows.Count().unwrap_or(0).max(0);

        let mut candidates: Vec<PathBuf> = Vec::new();
        for index in 0..count {
            let mut variant = VARIANT::default();
            let inner = &mut *variant.Anonymous.Anonymous;
            inner.vt = VT_I4;
            inner.Anonymous.lVal = index;

            let Ok(dispatch) = shell_windows.Item(&variant) else {
                continue;
            };
            let Ok(browser) = dispatch.cast::<IWebBrowser2>() else {
                continue;
            };
            let Ok(browser_hwnd) = browser.HWND() else {
                continue;
            };
            if browser_hwnd.0 as isize != foreground.0 as isize {
                continue;
            }

            let mut tab_matches = false;
            if let (Some(active_tab), Ok(provider)) =
                (active_tab, browser.cast::<IServiceProvider>())
            {
                if let Ok(shell_browser) = provider.QueryService::<IShellBrowser>(&SID_STopLevelBrowser)
                {
                    if let Ok(view_hwnd) = shell_browser.GetWindow() {
                        if view_hwnd == active_tab {
                            tab_matches = true;
                        }
                    }
                }
            }

            let Ok(location) = browser.LocationURL() else {
                continue;
            };
            let Some(path) = location_to_path(&location) else {
                continue;
            };

            if tab_matches {
                return Ok(path);
            }
            if !candidates.contains(&path) {
                candidates.push(path);
            }
        }

        if candidates.len() == 1 {
            return Ok(candidates.remove(0));
        }
        if candidates.len() > 1 {
            return Err("检测到多个资源管理器标签页，但无法确定当前标签页。请切到目标标签页后重试。".to_owned());
        }
        Err("无法获取资源管理器当前文件夹（当前页面可能不是普通文件夹）。".to_owned())
    }
}

fn location_to_path(location: &BSTR) -> Option<PathBuf> {
    let url = location.to_string();
    if url.is_empty() {
        return None;
    }
    if !url.starts_with("file:") {
        return None;
    }
    let mut wide: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
    let mut buffer = vec![0u16; 32768];
    let mut capacity = buffer.len() as u32;
    unsafe {
        PathCreateFromUrlW(
            windows::core::PCWSTR(wide.as_mut_ptr()),
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut capacity,
            0,
        )
        .ok()?;
    }
    let length = buffer.iter().position(|&ch| ch == 0).unwrap_or(buffer.len());
    let path = String::from_utf16_lossy(&buffer[..length]);
    let path = PathBuf::from(path);
    path.is_dir().then_some(path)
}
