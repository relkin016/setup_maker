//! Simple progress window used by the uninstaller (Inno "Uninstall Status").

use super::*;
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

thread_local! {
    static BAR: Cell<HWND> = const { Cell::new(std::ptr::null_mut()) };
    static WHITE: Cell<HBRUSH> = const { Cell::new(std::ptr::null_mut()) };
    static SC: Cell<f32> = const { Cell::new(1.0) };
}

unsafe extern "system" fn proc_(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            let s = SC.with(|c| c.get());
            let mut rc: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut rc);
            let top = RECT { left: 0, top: 0, right: rc.right, bottom: (58.0 * s) as i32 };
            FillRect(dc, &top, WHITE.with(|c| c.get()));
            let mut line = RECT { left: 0, top: top.bottom, right: rc.right, bottom: top.bottom + 2 };
            DrawEdge(dc, &mut line, EDGE_ETCHED, BF_TOP);
            EndPaint(hwnd, &ps);
            return 0;
        }
        WM_CTLCOLORSTATIC => {
            let ctl = lp as HWND;
            let mut r: RECT = std::mem::zeroed();
            GetWindowRect(ctl, &mut r);
            let mut pt = POINT { x: r.left, y: r.top };
            ScreenToClient(hwnd, &mut pt);
            if (pt.y as f32) < 58.0 * SC.with(|c| c.get()) {
                SetBkColor(wp as HDC, 0xffffff);
                return WHITE.with(|c| c.get()) as LRESULT;
            }
        }
        WM_CLOSE => return 0,
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

/// Show the window while `work` runs on a worker thread.
pub fn run<F>(title: &str, heading: &str, label: &str, cancel: &str, prog: Arc<AtomicU32>, work: F) -> bool
where
    F: FnOnce(Arc<AtomicU32>) -> bool + Send + 'static,
{
    init_common_controls();
    register_class("SmkProgress", Some(proc_));
    let s = dpi() as f32 / 96.0;
    SC.with(|c| c.set(s));
    let sc = |v: i32| (v as f32 * s) as i32;
    let f = make_fonts();
    unsafe {
        WHITE.with(|c| c.set(CreateSolidBrush(0xffffff)));
        let style = WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let mut r = RECT { left: 0, top: 0, right: sc(497), bottom: sc(200) };
        AdjustWindowRectEx(&mut r, style, 0, 0);
        let cn = wide("SmkProgress");
        let t = wide(title);
        let hwnd = CreateWindowExW(0, cn.as_ptr(), t.as_ptr(), style, 0, 0, r.right - r.left, r.bottom - r.top, std::ptr::null_mut(), std::ptr::null_mut(), hinst(), std::ptr::null());
        center(hwnd, std::ptr::null_mut());
        child(hwnd, "STATIC", heading, WS_VISIBLE | SS_NOPREFIX as u32, 0, sc(24), sc(10), sc(400), sc(16), -1, f.bold);
        child(hwnd, "STATIC", label, WS_VISIBLE | SS_NOPREFIX as u32, 0, sc(40), sc(28), sc(400), sc(28), -1, f.normal);
        let bar = child(hwnd, "msctls_progress32", "", WS_VISIBLE, 0, sc(40), sc(90), sc(417), sc(21), -1, f.normal);
        SendMessageW(bar, PBM_SETRANGE32, 0, 1000);
        BAR.with(|c| c.set(bar));
        let btn = child(hwnd, "BUTTON", cancel, WS_VISIBLE, 0, sc(414), sc(167), sc(75), sc(23), 2, f.normal);
        EnableWindow(btn, 0);
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);

        let done = Arc::new(AtomicBool::new(false));
        let result = Arc::new(AtomicBool::new(false));
        let (d2, r2, p2) = (done.clone(), result.clone(), prog.clone());
        std::thread::spawn(move || {
            let ok = work(p2);
            r2.store(ok, Ordering::SeqCst);
            d2.store(true, Ordering::SeqCst);
        });
        let mut msg: MSG = std::mem::zeroed();
        while !done.load(Ordering::SeqCst) {
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            SendMessageW(bar, PBM_SETPOS, prog.load(Ordering::Relaxed) as usize, 0);
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        SendMessageW(bar, PBM_SETPOS, 1000, 0);
        DestroyWindow(hwnd);
        result.load(Ordering::SeqCst)
    }
}
