//! Win32 selection uses physical virtual-desktop pixels throughout, including negative origins.
use anyhow::{Context, Result, ensure};
use image::{DynamicImage, RgbImage};
use std::{
    ffi::c_void,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    sync::OnceLock,
};
use tokio_util::sync::CancellationToken;
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{JobObjects::*, LibraryLoader::GetModuleHandleW, Threading::*},
    UI::{HiDpi::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

pub struct ProcessJob(usize);
impl ProcessJob {
    pub fn attach(pid: u32) -> Result<Self> {
        // Job handle is owned here; the child dies when this handle or its parent closes.
        unsafe {
            let job = CreateJobObjectW(null(), null());
            ensure!(!job.is_null(), "无法创建后台进程组");
            let owner = Self(job as usize);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            ensure!(
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as *const c_void,
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32
                ) != 0,
                "无法配置后台进程组"
            );
            let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
            ensure!(!process.is_null(), "无法关联后台进程");
            let result = AssignProcessToJobObject(job, process);
            CloseHandle(process);
            ensure!(
                result != 0,
                "无法管理后台进程：{}",
                std::io::Error::last_os_error()
            );
            Ok(owner)
        }
    }
}
impl Drop for ProcessJob {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0 as HANDLE);
        }
    }
}

// Hide on the UI thread, then wait for DWM to present the desktop without our window.
pub fn hide_for_capture(handle: usize, hide: impl FnOnce() -> Result<()>) -> Result<()> {
    unsafe {
        use windows_sys::Win32::Graphics::Dwm::{
            DWMWA_TRANSITIONS_FORCEDISABLED, DwmFlush, DwmSetWindowAttribute,
        };
        let window = handle as HWND;
        let disabled: i32 = 1;
        let _ = DwmSetWindowAttribute(
            window,
            DWMWA_TRANSITIONS_FORCEDISABLED as u32,
            &disabled as *const _ as *const c_void,
            size_of::<i32>() as u32,
        );
        // Tauri must update its visibility state as well as the HWND.
        hide()?;
        ensure!(IsWindowVisible(window) == 0, "无法隐藏截图窗口，请重试");
        let _ = DwmFlush();
    }
    Ok(())
}

pub fn restore_capture_transitions(handle: usize) {
    unsafe {
        use windows_sys::Win32::Graphics::Dwm::{
            DWMWA_TRANSITIONS_FORCEDISABLED, DwmSetWindowAttribute,
        };
        let disabled: i32 = 0;
        let _ = DwmSetWindowAttribute(
            handle as HWND,
            DWMWA_TRANSITIONS_FORCEDISABLED as u32,
            &disabled as *const _ as *const c_void,
            size_of::<i32>() as u32,
        );
    }
}

struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    pixels: *mut u8,
    length: usize,
}
impl Surface {
    unsafe fn new(screen: HDC, width: i32, height: i32) -> Result<Self> {
        unsafe {
            let dc = CreateCompatibleDC(screen);
            ensure!(!dc.is_null(), "无法创建截图缓冲区");
            let mut info: BITMAPINFO = zeroed();
            info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = width;
            info.bmiHeader.biHeight = -height;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB;
            let mut bits = null_mut();
            let bitmap = CreateDIBSection(screen, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if bitmap.is_null() || bits.is_null() {
                DeleteDC(dc);
                anyhow::bail!("无法分配截图缓冲区");
            }
            let previous = SelectObject(dc, bitmap);
            Ok(Self {
                dc,
                bitmap,
                previous,
                pixels: bits.cast(),
                length: width as usize * height as usize * 4,
            })
        }
    }
}
impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}
struct DesktopDc(HDC);
impl Drop for DesktopDc {
    fn drop(&mut self) {
        unsafe {
            ReleaseDC(null_mut(), self.0);
        }
    }
}
struct Capture {
    width: i32,
    height: i32,
    left: i32,
    top: i32,
    original: Surface,
    dimmed: Surface,
    frame: Surface,
    font: HFONT,
    pen: HPEN,
    dpi: i32,
    start: Option<POINT>,
    cursor: POINT,
    result: Option<RECT>,
    cancel: CancellationToken,
}
impl Drop for Capture {
    fn drop(&mut self) {
        unsafe {
            DeleteObject(self.font);
            DeleteObject(self.pen);
        }
    }
}
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

pub fn select_region(cancel: CancellationToken) -> Result<Option<DynamicImage>> {
    unsafe {
        let previous = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let result = select_region_inner(cancel);
        if !previous.is_null() {
            SetThreadDpiAwarenessContext(previous);
        }
        result
    }
}
fn select_region_inner(cancel: CancellationToken) -> Result<Option<DynamicImage>> {
    let mut state = Box::new(capture_desktop(cancel)?);
    static CLASS: OnceLock<Vec<u16>> = OnceLock::new();
    let class = CLASS.get_or_init(|| wide("BlinkOCRPhysicalSelection"));
    unsafe {
        let instance = GetModuleHandleW(null());
        let mut window_class: WNDCLASSW = zeroed();
        window_class.lpfnWndProc = Some(window_proc);
        window_class.hInstance = instance;
        window_class.hCursor = LoadCursorW(null_mut(), IDC_CROSS);
        window_class.lpszClassName = class.as_ptr();
        RegisterClassW(&window_class);
        let window = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class.as_ptr(),
            wide("BlinkOCR · 框选").as_ptr(),
            WS_POPUP,
            state.left,
            state.top,
            state.width,
            state.height,
            null_mut(),
            null_mut(),
            instance,
            &mut *state as *mut Capture as *const c_void,
        );
        ensure!(
            !window.is_null(),
            "无法打开屏幕框选：{}",
            std::io::Error::last_os_error()
        );
        state.dpi = GetDpiForWindow(window).max(96) as i32;
        state.font = CreateFontW(
            -(15 * state.dpi / 96),
            0,
            0,
            0,
            500,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            0,
            wide("Microsoft YaHei UI").as_ptr(),
        );
        state.pen = CreatePen(PS_SOLID, (2 * state.dpi / 96).max(2), 0x00B9AA27);
        ShowWindow(window, SW_SHOW);
        SetForegroundWindow(window);
        SetFocus(window);
        SetTimer(window, 1, 30, None);
        let mut message: MSG = zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    let Some(rect) = state.result else {
        return Ok(None);
    };
    let source =
        unsafe { std::slice::from_raw_parts(state.original.pixels, state.original.length) };
    let mut rgb =
        Vec::with_capacity(((rect.right - rect.left) * (rect.bottom - rect.top) * 3) as usize);
    for y in rect.top..rect.bottom {
        for x in rect.left..rect.right {
            let offset = ((y * state.width + x) * 4) as usize;
            rgb.extend_from_slice(&[source[offset + 2], source[offset + 1], source[offset]]);
        }
    }
    let image = RgbImage::from_raw(
        (rect.right - rect.left) as u32,
        (rect.bottom - rect.top) as u32,
        rgb,
    )
    .context("截图数据有误")?;
    Ok(Some(DynamicImage::ImageRgb8(image)))
}
fn capture_desktop(cancel: CancellationToken) -> Result<Capture> {
    unsafe {
        let left = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let top = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let width = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let height = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        ensure!(width > 0 && height > 0, "无法获取显示器范围");
        let screen = DesktopDc(GetDC(null_mut()));
        ensure!(!screen.0.is_null(), "无法读取桌面");
        let original = Surface::new(screen.0, width, height)?;
        ensure!(
            BitBlt(
                original.dc,
                0,
                0,
                width,
                height,
                screen.0,
                left,
                top,
                SRCCOPY | CAPTUREBLT
            ) != 0,
            "屏幕截图失败，请重新尝试"
        );
        GdiFlush();
        let dimmed = Surface::new(screen.0, width, height)?;
        let source = std::slice::from_raw_parts(original.pixels, original.length);
        let target = std::slice::from_raw_parts_mut(dimmed.pixels, dimmed.length);
        for (source, target) in source.chunks_exact(4).zip(target.chunks_exact_mut(4)) {
            for index in 0..3 {
                target[index] = (source[index] as u16 * 48 / 100) as u8;
            }
            target[3] = 0;
        }
        let frame = Surface::new(screen.0, width, height)?;
        Ok(Capture {
            width,
            height,
            left,
            top,
            original,
            dimmed,
            frame,
            font: null_mut(),
            pen: null_mut(),
            dpi: 96,
            start: None,
            cursor: POINT { x: 0, y: 0 },
            result: None,
            cancel,
        })
    }
}
fn selection(state: &Capture) -> Option<RECT> {
    state.start.map(|start| RECT {
        left: start.x.min(state.cursor.x).clamp(0, state.width),
        top: start.y.min(state.cursor.y).clamp(0, state.height),
        right: start.x.max(state.cursor.x).clamp(0, state.width),
        bottom: start.y.max(state.cursor.y).clamp(0, state.height),
    })
}
fn tooltip(state: &Capture) -> (Vec<u16>, RECT) {
    let text = wide(&if let Some(rect) = selection(state) {
        format!(
            "{} × {}  ·  松开识别  /  Esc 取消",
            rect.right - rect.left,
            rect.bottom - rect.top
        )
    } else {
        "拖动框选文字区域  ·  Esc 取消".into()
    });
    let mut size: SIZE = unsafe { zeroed() };
    unsafe {
        let previous = SelectObject(state.frame.dc, state.font);
        GetTextExtentPoint32W(
            state.frame.dc,
            text.as_ptr(),
            text.len() as i32 - 1,
            &mut size,
        );
        SelectObject(state.frame.dc, previous);
    }
    let padding = 10 * state.dpi / 96;
    let x = (state.cursor.x + 20 * state.dpi / 96)
        .clamp(padding, (state.width - size.cx - padding).max(padding));
    let y = (state.cursor.y + 28 * state.dpi / 96)
        .clamp(padding, (state.height - size.cy - padding).max(padding));
    (
        text,
        RECT {
            left: x,
            top: y,
            right: x + size.cx + padding,
            bottom: y + size.cy + padding,
        },
    )
}
fn paint_bounds(state: &Capture) -> RECT {
    let (_, tip) = tooltip(state);
    if let Some(area) = selection(state) {
        let border = 4 * state.dpi / 96;
        RECT {
            left: tip.left.min(area.left - border),
            top: tip.top.min(area.top - border),
            right: tip.right.max(area.right + border),
            bottom: tip.bottom.max(area.bottom + border),
        }
    } else {
        tip
    }
}
// This pointer belongs to Box<Capture> until this thread's message loop exits.
#[allow(unsafe_op_in_unsafe_fn)]
unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = &*(lparam as *const CREATESTRUCTW);
        SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize);
        return 1;
    }
    let pointer = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut Capture;
    if pointer.is_null() {
        return DefWindowProcW(window, message, wparam, lparam);
    }
    let state = &mut *pointer;
    match message {
        WM_LBUTTONDOWN | WM_MOUSEMOVE | WM_LBUTTONUP => {
            // Use each queued event's client coordinates, not a newer global cursor position.
            let cursor = POINT {
                x: (lparam as u16 as i16) as i32,
                y: ((lparam >> 16) as u16 as i16) as i32,
            };
            if message == WM_MOUSEMOVE && cursor.x == state.cursor.x && cursor.y == state.cursor.y {
                return 0;
            }
            let before = paint_bounds(state);
            state.cursor = cursor;
            if message == WM_LBUTTONDOWN {
                state.start = Some(cursor);
                SetCapture(window);
            }
            if message == WM_LBUTTONUP {
                let result = selection(state);
                ReleaseCapture();
                if let Some(rect) = result
                    && rect.right - rect.left >= 3
                    && rect.bottom - rect.top >= 3
                {
                    state.result = Some(rect);
                    DestroyWindow(window);
                    return 0;
                }
                state.start = None;
            }
            let after = paint_bounds(state);
            let mut dirty: RECT = zeroed();
            UnionRect(&mut dirty, &before, &after);
            InvalidateRect(window, &dirty, 0);
            0
        }
        WM_RBUTTONDOWN | WM_CLOSE => {
            DestroyWindow(window);
            0
        }
        WM_KEYDOWN if wparam == VK_ESCAPE as usize => {
            DestroyWindow(window);
            0
        }
        WM_TIMER if state.cancel.is_cancelled() => {
            DestroyWindow(window);
            0
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut paint: PAINTSTRUCT = zeroed();
            let dc = BeginPaint(window, &mut paint);
            let dirty = paint.rcPaint;
            let saved = SaveDC(state.frame.dc);
            IntersectClipRect(
                state.frame.dc,
                dirty.left,
                dirty.top,
                dirty.right,
                dirty.bottom,
            );
            BitBlt(
                state.frame.dc,
                dirty.left,
                dirty.top,
                dirty.right - dirty.left,
                dirty.bottom - dirty.top,
                state.dimmed.dc,
                dirty.left,
                dirty.top,
                SRCCOPY,
            );
            if let Some(rect) = selection(state) {
                BitBlt(
                    state.frame.dc,
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    state.original.dc,
                    rect.left,
                    rect.top,
                    SRCCOPY,
                );
                SelectObject(state.frame.dc, state.pen);
                SelectObject(state.frame.dc, GetStockObject(NULL_BRUSH));
                Rectangle(state.frame.dc, rect.left, rect.top, rect.right, rect.bottom);
            }
            SelectObject(state.frame.dc, state.font);
            SetBkMode(state.frame.dc, TRANSPARENT as i32);
            SetTextColor(state.frame.dc, 0x00FFFFFF);
            let (text, tip) = tooltip(state);
            TextOutW(
                state.frame.dc,
                tip.left,
                tip.top,
                text.as_ptr(),
                text.len() as i32 - 1,
            );
            RestoreDC(state.frame.dc, saved);
            // Present one completed frame; the desktop never sees the intermediate dimmed frame.
            BitBlt(
                dc,
                dirty.left,
                dirty.top,
                dirty.right - dirty.left,
                dirty.bottom - dirty.top,
                state.frame.dc,
                dirty.left,
                dirty.top,
                SRCCOPY,
            );
            EndPaint(window, &paint);
            0
        }
        WM_DESTROY => {
            KillTimer(window, 1);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(window, message, wparam, lparam),
    }
}
