//! Win32 UI: Inno-style wizard, small dialogs, uninstall progress window.
#![allow(dead_code)]

pub mod dialogs;
pub mod progress;
pub mod wizard;

use crate::w::wide;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;
pub use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};

pub const SS_ICON: u32 = 0x3;
pub const SS_NOPREFIX: u32 = 0x80;
pub const SS_PATHELLIPSIS: u32 = 0x8000;

pub struct Fonts {
    pub normal: HFONT,
    pub bold: HFONT,
    pub big: HFONT,
}

pub fn init_common_controls() {
    unsafe {
        let icc = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_PROGRESS_CLASS | ICC_TREEVIEW_CLASSES | ICC_STANDARD_CLASSES | ICC_WIN95_CLASSES,
        };
        InitCommonControlsEx(&icc);
        let rich = wide("Msftedit.dll");
        windows_sys::Win32::System::LibraryLoader::LoadLibraryW(rich.as_ptr());
    }
}

pub fn dpi() -> i32 {
    unsafe {
        let dc = GetDC(std::ptr::null_mut());
        let d = GetDeviceCaps(dc, LOGPIXELSY as i32);
        ReleaseDC(std::ptr::null_mut(), dc);
        if d <= 0 {
            96
        } else {
            d
        }
    }
}

pub fn make_fonts() -> Fonts {
    unsafe {
        let mut ncm: NONCLIENTMETRICSW = std::mem::zeroed();
        ncm.cbSize = std::mem::size_of::<NONCLIENTMETRICSW>() as u32;
        SystemParametersInfoW(SPI_GETNONCLIENTMETRICS, ncm.cbSize, &mut ncm as *mut _ as *mut _, 0);
        let mut lf = ncm.lfMessageFont;
        if lf.lfHeight == 0 {
            lf.lfHeight = -(9 * dpi() / 72);
        }
        let normal = CreateFontIndirectW(&lf);
        let mut b = lf;
        b.lfWeight = FW_BOLD as i32;
        let bold = CreateFontIndirectW(&b);
        let mut g = b;
        g.lfHeight = (lf.lfHeight as f32 * 1.45) as i32;
        let big = CreateFontIndirectW(&g);
        Fonts { normal, bold, big }
    }
}

pub fn set_text(h: HWND, s: &str) {
    let t = wide(&s.replace("\r\n", "\n").replace('\n', "\r\n"));
    unsafe {
        SetWindowTextW(h, t.as_ptr());
    }
}

pub fn get_text(h: HWND) -> String {
    unsafe {
        let n = GetWindowTextLengthW(h);
        let mut b = vec![0u16; n as usize + 1];
        GetWindowTextW(h, b.as_mut_ptr(), b.len() as i32);
        crate::w::from_wide_buf(&b)
    }
}

pub fn show(h: HWND, v: bool) {
    unsafe {
        ShowWindow(h, if v { SW_SHOW } else { SW_HIDE });
    }
}

pub fn enable(h: HWND, v: bool) {
    unsafe {
        EnableWindow(h, v as i32);
    }
}

pub fn set_font(h: HWND, f: HFONT) {
    unsafe {
        SendMessageW(h, WM_SETFONT, f as usize, 1);
    }
}

pub fn is_checked(h: HWND) -> bool {
    unsafe { SendMessageW(h, BM_GETCHECK, 0, 0) == BST_CHECKED as isize }
}

pub fn set_checked(h: HWND, v: bool) {
    unsafe {
        SendMessageW(h, BM_SETCHECK, if v { BST_CHECKED as usize } else { BST_UNCHECKED as usize }, 0);
    }
}

pub fn hinst() -> HINSTANCE {
    unsafe { windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null()) }
}

pub fn app_icon(size: i32) -> HICON {
    unsafe { LoadImageW(hinst(), 1 as _, IMAGE_ICON, size, size, LR_DEFAULTCOLOR) as HICON }
}

/// Create a child control.
pub fn child(parent: HWND, class: &str, text: &str, style: u32, ex: u32, x: i32, y: i32, w: i32, h: i32, id: i32, font: HFONT) -> HWND {
    let c = wide(class);
    let t = wide(&text.replace("\r\n", "\n").replace('\n', "\r\n"));
    unsafe {
        let hwnd = CreateWindowExW(ex, c.as_ptr(), t.as_ptr(), WS_CHILD | style, x, y, w, h, parent, id as isize as HMENU, hinst(), std::ptr::null());
        set_font(hwnd, font);
        hwnd
    }
}

/// 32-bit top-down DIB section from BGRA pixels.
pub struct Image {
    pub bmp: HBITMAP,
    pub w: i32,
    pub h: i32,
}

impl Image {
    pub fn from_resource(data: &[u8]) -> Option<Image> {
        if data.len() < 8 {
            return None;
        }
        let w = u32::from_le_bytes(data[0..4].try_into().ok()?) as i32;
        let h = u32::from_le_bytes(data[4..8].try_into().ok()?) as i32;
        let px = &data[8..];
        if px.len() < (w * h * 4) as usize || w <= 0 || h <= 0 {
            return None;
        }
        Self::from_bgra(w, h, px)
    }

    pub fn from_bgra(w: i32, h: i32, px: &[u8]) -> Option<Image> {
        unsafe {
            let mut bi: BITMAPINFO = std::mem::zeroed();
            bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bi.bmiHeader.biWidth = w;
            bi.bmiHeader.biHeight = -h;
            bi.bmiHeader.biPlanes = 1;
            bi.bmiHeader.biBitCount = 32;
            bi.bmiHeader.biCompression = BI_RGB;
            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let bmp = CreateDIBSection(std::ptr::null_mut(), &bi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
            if bmp.is_null() || bits.is_null() {
                return None;
            }
            std::ptr::copy_nonoverlapping(px.as_ptr(), bits as *mut u8, (w * h * 4) as usize);
            Some(Image { bmp, w, h })
        }
    }

    pub fn draw(&self, dc: HDC, x: i32, y: i32, w: i32, h: i32) {
        unsafe {
            let mem = CreateCompatibleDC(dc);
            let old = SelectObject(mem, self.bmp as HGDIOBJ);
            SetStretchBltMode(dc, HALFTONE);
            SetBrushOrgEx(dc, 0, 0, std::ptr::null_mut());
            StretchBlt(dc, x, y, w, h, mem, 0, 0, self.w, self.h, SRCCOPY);
            SelectObject(mem, old);
            DeleteDC(mem);
        }
    }
}

/// Default welcome image: vertical gradient in the accent colour.
pub fn default_large_image(accent: u32) -> Option<Image> {
    let (w, h) = (164, 314);
    let (r0, g0, b0) = ((accent >> 16) & 255, (accent >> 8) & 255, accent & 255);
    let mut px = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        let t = y as f32 / h as f32;
        let mix = |c: u32, k: f32| ((c as f32) * (1.0 - k) + 255.0 * k * 0.15).min(255.0) as u8;
        let dark = |c: u32| ((c as f32) * (0.55 + 0.45 * (1.0 - t))) as u8;
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let k = x as f32 / w as f32 * 0.4;
            px[i] = mix(dark(b0) as u32, k);
            px[i + 1] = mix(dark(g0) as u32, k);
            px[i + 2] = mix(dark(r0) as u32, k);
            px[i + 3] = 255;
        }
    }
    Image::from_bgra(w, h, &px)
}

pub fn parse_color(s: &str) -> Option<u32> {
    let s = s.trim().trim_start_matches('#');
    if s.len() == 6 {
        u32::from_str_radix(s, 16).ok()
    } else {
        None
    }
}

/// Run a modal message loop until `done()` is true.
pub fn modal_loop(dlg: HWND, done: &dyn Fn() -> bool) {
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while !done() && GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            if IsDialogMessageW(dlg, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}

pub fn center(hwnd: HWND, owner: HWND) {
    unsafe {
        let mut r: RECT = std::mem::zeroed();
        GetWindowRect(hwnd, &mut r);
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let mut o: RECT = std::mem::zeroed();
        if owner.is_null() || IsWindowVisible(owner) == 0 {
            SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut o as *mut _ as *mut _, 0);
        } else {
            GetWindowRect(owner, &mut o);
        }
        let x = o.left + ((o.right - o.left) - w) / 2;
        let y = o.top + ((o.bottom - o.top) - h) / 2;
        SetWindowPos(hwnd, std::ptr::null_mut(), x.max(0), y.max(0), 0, 0, SWP_NOSIZE | SWP_NOZORDER);
    }
}

pub fn register_class(name: &str, proc_: WNDPROC) {
    let cn = wide(name);
    unsafe {
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: proc_,
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst(),
            hIcon: app_icon(32),
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_BTNFACE + 1) as isize as HBRUSH,
            lpszMenuName: std::ptr::null(),
            lpszClassName: cn.as_ptr(),
            hIconSm: app_icon(16),
        };
        RegisterClassExW(&wc);
    }
}

/// Browse for a folder; returns the chosen path.
pub fn browse_folder(owner: HWND, title: &str, initial: &str) -> Option<String> {
    use windows_sys::Win32::UI::Shell::*;
    unsafe extern "system" fn cb(hwnd: HWND, msg: u32, _l: LPARAM, data: LPARAM) -> i32 {
        if msg == BFFM_INITIALIZED {
            SendMessageW(hwnd, BFFM_SETSELECTIONW, 1, data);
        }
        0
    }
    let t = wide(title);
    let init = wide(initial);
    let mut name = vec![0u16; 260];
    unsafe {
        let bi = BROWSEINFOW {
            hwndOwner: owner,
            pidlRoot: std::ptr::null_mut(),
            pszDisplayName: name.as_mut_ptr(),
            lpszTitle: t.as_ptr(),
            ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE | BIF_EDITBOX,
            lpfn: Some(cb),
            lParam: init.as_ptr() as LPARAM,
            iImage: 0,
        };
        let pidl = SHBrowseForFolderW(&bi);
        if pidl.is_null() {
            return None;
        }
        let mut path = vec![0u16; 1024];
        let ok = SHGetPathFromIDListW(pidl, path.as_mut_ptr());
        windows_sys::Win32::System::Com::CoTaskMemFree(pidl as *const _);
        if ok != 0 {
            Some(crate::w::from_wide_buf(&path))
        } else {
            None
        }
    }
}
