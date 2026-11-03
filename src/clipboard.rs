//! Native Windows Clipboard integration for Files (`CF_HDROP`).
//! Supports Copy (`DROPEFFECT_COPY`) and Cut (`DROPEFFECT_MOVE`),
//! allowing files to be directly pasted or moved in Windows File Explorer,
//! and also sets `CF_UNICODETEXT` for plaintext pasting into editors.

#[cfg(windows)]
mod win_clipboard {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr;

    type BOOL = i32;
    type UINT = u32;
    type DWORD = u32;
    type HGLOBAL = *mut c_void;

    const CF_UNICODETEXT: UINT = 13;
    const CF_HDROP: UINT = 15;
    const GHND: UINT = 0x0042; // GMEM_MOVEABLE | GMEM_ZEROINIT

    const DROPEFFECT_COPY: DWORD = 1;
    const DROPEFFECT_MOVE: DWORD = 2;

    #[repr(C)]
    struct DROPFILES {
        p_files: DWORD, // Offset of the file list from beginning of structure
        pt_x: i32,
        pt_y: i32,
        f_nc: BOOL,
        f_wide: BOOL,   // 1 if unicode
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn OpenClipboard(hWndNewOwner: *mut c_void) -> BOOL;
        fn EmptyClipboard() -> BOOL;
        fn CloseClipboard() -> BOOL;
        fn SetClipboardData(uFormat: UINT, hMem: HGLOBAL) -> HGLOBAL;
        fn RegisterClipboardFormatW(lpszFormat: *const u16) -> UINT;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GlobalAlloc(uFlags: UINT, dwBytes: usize) -> HGLOBAL;
        fn GlobalLock(hMem: HGLOBAL) -> *mut c_void;
        fn GlobalUnlock(hMem: HGLOBAL) -> BOOL;
        fn GlobalFree(hMem: HGLOBAL) -> HGLOBAL;
    }

    pub fn set_files_to_clipboard(paths: &[String], is_cut: bool) -> bool {
        if paths.is_empty() {
            return false;
        }

        let dropfiles_header_size = std::mem::size_of::<DROPFILES>();
        let mut wide_chars: Vec<u16> = Vec::new();

        for p in paths {
            let encoded: Vec<u16> = Path::new(p)
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            wide_chars.extend(encoded);
        }
        wide_chars.push(0); // Final double-null terminator

        let dropfiles_total_size = dropfiles_header_size + wide_chars.len() * 2;

        unsafe {
            // Allocate memory for DROPFILES
            let h_drop = GlobalAlloc(GHND, dropfiles_total_size);
            if h_drop.is_null() {
                return false;
            }

            let p_drop = GlobalLock(h_drop);
            if p_drop.is_null() {
                GlobalFree(h_drop);
                return false;
            }

            let dropfiles = DROPFILES {
                p_files: dropfiles_header_size as DWORD,
                pt_x: 0,
                pt_y: 0,
                f_nc: 0,
                f_wide: 1, // UTF-16
            };
            ptr::copy_nonoverlapping(
                &dropfiles as *const DROPFILES as *const u8,
                p_drop as *mut u8,
                dropfiles_header_size,
            );

            let dest_paths = (p_drop as *mut u8).add(dropfiles_header_size) as *mut u16;
            ptr::copy_nonoverlapping(wide_chars.as_ptr(), dest_paths, wide_chars.len());

            GlobalUnlock(h_drop);

            // Allocate memory for Preferred DropEffect (Copy vs Cut)
            let drop_effect_format_name: Vec<u16> = "Preferred DropEffect\0".encode_utf16().collect();
            let cf_drop_effect = RegisterClipboardFormatW(drop_effect_format_name.as_ptr());

            let effect_val: DWORD = if is_cut {
                DROPEFFECT_MOVE
            } else {
                DROPEFFECT_COPY
            };

            let mut h_effect = GlobalAlloc(GHND, std::mem::size_of::<DWORD>());
            if !h_effect.is_null() {
                let p_effect = GlobalLock(h_effect);
                if !p_effect.is_null() {
                    ptr::copy_nonoverlapping(
                        &effect_val as *const DWORD as *const u8,
                        p_effect as *mut u8,
                        std::mem::size_of::<DWORD>(),
                    );
                    GlobalUnlock(h_effect);
                } else {
                    GlobalFree(h_effect);
                    h_effect = ptr::null_mut();
                }
            }

            // Allocate memory for CF_UNICODETEXT fallback (for pasting in text editors)
            let text_content = paths.join("\r\n");
            let text_wide: Vec<u16> = text_content
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let mut h_text = GlobalAlloc(GHND, text_wide.len() * 2);
            if !h_text.is_null() {
                let p_text = GlobalLock(h_text);
                if !p_text.is_null() {
                    ptr::copy_nonoverlapping(
                        text_wide.as_ptr(),
                        p_text as *mut u16,
                        text_wide.len(),
                    );
                    GlobalUnlock(h_text);
                } else {
                    GlobalFree(h_text);
                    h_text = ptr::null_mut();
                }
            }

            // Open clipboard with retry loop to handle momentary lock by clipboard viewers
            let mut opened = false;
            for _ in 0..6 {
                if OpenClipboard(ptr::null_mut()) != 0 {
                    opened = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(15));
            }

            if !opened {
                GlobalFree(h_drop);
                if !h_effect.is_null() {
                    GlobalFree(h_effect);
                }
                if !h_text.is_null() {
                    GlobalFree(h_text);
                }
                return false;
            }

            EmptyClipboard();

            // Set CF_HDROP
            let res_drop = SetClipboardData(CF_HDROP, h_drop);
            if res_drop.is_null() {
                GlobalFree(h_drop);
            }

            // Set Preferred DropEffect
            if cf_drop_effect != 0 && !h_effect.is_null() {
                let res_eff = SetClipboardData(cf_drop_effect, h_effect);
                if res_eff.is_null() {
                    GlobalFree(h_effect);
                }
            }

            // Set CF_UNICODETEXT
            if !h_text.is_null() {
                let res_txt = SetClipboardData(CF_UNICODETEXT, h_text);
                if res_txt.is_null() {
                    GlobalFree(h_text);
                }
            }

            CloseClipboard();
            !res_drop.is_null()
        }
    }
}

pub fn copy_or_cut_files(paths: &[String], is_cut: bool) -> bool {
    #[cfg(windows)]
    {
        win_clipboard::set_files_to_clipboard(paths, is_cut)
    }
    #[cfg(not(windows))]
    {
        let _ = (paths, is_cut);
        false
    }
}
