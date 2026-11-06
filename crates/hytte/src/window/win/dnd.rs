//! Drag and drop: the shelf as a drop target, dragging items out.

use super::*;

// ------------------------------------------------------------ drop target

#[windows::core::implement(IDropTarget)]
pub(super) struct Target {
    shared: Arc<Shared>,
}

pub(super) fn query(data: &IDataObject, fmt: u16) -> FORMATETC {
    let _ = data;
    FORMATETC {
        cfFormat: fmt,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

pub(super) const CF_UNICODETEXT: u16 = 13;
pub(super) const CF_HDROP: u16 = 15;

pub(super) fn accepts(data: &IDataObject) -> bool {
    unsafe {
        data.QueryGetData(&query(data, CF_HDROP)).is_ok()
            || data.QueryGetData(&query(data, CF_UNICODETEXT)).is_ok()
    }
}

pub(super) fn extract(data: &IDataObject) -> DropJob {
    let mut job = DropJob {
        paths: vec![],
        text: None,
        op: None,
    };
    unsafe {
        if let Ok(mut stg) = data.GetData(&query(data, CF_HDROP)) {
            let h = windows::Win32::UI::Shell::HDROP(stg.u.hGlobal.0);
            let n = DragQueryFileW(h, 0xFFFF_FFFF, None);
            for i in 0..n {
                let len = DragQueryFileW(h, i, None) as usize;
                let mut buf = vec![0u16; len + 1];
                let got = DragQueryFileW(h, i, Some(&mut buf)) as usize;
                job.paths.push(String::from_utf16_lossy(&buf[..got]).into());
            }
            ReleaseStgMedium(&mut stg);
        }
        if job.paths.is_empty() {
            if let Ok(mut stg) = data.GetData(&query(data, CF_UNICODETEXT)) {
                let h = stg.u.hGlobal;
                let p = GlobalLock(h) as *const u16;
                if !p.is_null() {
                    let mut len = 0;
                    while *p.add(len) != 0 && len < 4 * 1024 * 1024 {
                        len += 1;
                    }
                    job.text = Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, len)));
                    let _ = GlobalUnlock(h);
                }
                ReleaseStgMedium(&mut stg);
            }
        }
    }
    job
}

#[allow(non_snake_case)]
impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(
        &self,
        data: windows::core::Ref<IDataObject>,
        _k: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let ok = data.as_ref().is_some_and(accepts);
        unsafe {
            if !effect.is_null() {
                *effect = if ok { DROPEFFECT_COPY } else { DROPEFFECT_NONE };
            }
        }
        if ok {
            push_event(&self.shared, UiEvent::DragEnter);
        }
        Ok(())
    }
    fn DragOver(
        &self,
        _k: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        unsafe {
            if !effect.is_null() && *effect != DROPEFFECT_NONE {
                *effect = DROPEFFECT_COPY;
            }
        }
        Ok(())
    }
    fn DragLeave(&self) -> windows::core::Result<()> {
        push_event(&self.shared, UiEvent::DragLeave);
        Ok(())
    }
    fn Drop(
        &self,
        data: windows::core::Ref<IDataObject>,
        _k: MODIFIERKEYS_FLAGS,
        _pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        unsafe {
            if !effect.is_null() {
                *effect = DROPEFFECT_COPY;
            }
        }
        push_event(&self.shared, UiEvent::DragLeave);
        if let Some(d) = data.as_ref() {
            let job = extract(d);
            if !job.paths.is_empty() || job.text.is_some() {
                push_event(&self.shared, UiEvent::ShelfAdd(job));
            }
        }
        Ok(())
    }
}

pub(super) fn register_drop_target(hwnd: HWND, shared: Arc<Shared>) {
    unsafe {
        let t: IDropTarget = Target { shared }.into();
        let _ = RegisterDragDrop(hwnd, &t);
    }
}

// --------------------------------------------------------------- drag out

#[windows::core::implement(IDropSource)]
pub(super) struct Source;

#[allow(non_snake_case)]
impl IDropSource_Impl for Source_Impl {
    fn QueryContinueDrag(
        &self,
        escape: windows::core::BOOL,
        keys: MODIFIERKEYS_FLAGS,
    ) -> windows::core::HRESULT {
        if escape.as_bool() {
            DRAGDROP_S_CANCEL
        } else if keys.0 & 1 == 0 {
            DRAGDROP_S_DROP
        } else {
            windows::core::HRESULT(0)
        }
    }
    fn GiveFeedback(&self, _effect: DROPEFFECT) -> windows::core::HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

/// Start an OLE drag of one file and return the effect the target applied.
pub(super) fn shelf_drag(path: &std::path::Path) -> DROPEFFECT {
    use windows::core::{Interface, HSTRING};
    use windows::Win32::UI::Shell::{BHID_DataObject, IShellItem, SHCreateItemFromParsingName};
    unsafe {
        let Ok(item) =
            SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(path.as_os_str()), None)
        else {
            return DROPEFFECT_NONE;
        };
        let Ok(data) = item.BindToHandler::<_, IDataObject>(None, &BHID_DataObject) else {
            return DROPEFFECT_NONE;
        };
        let src: IDropSource = Source.into();
        let mut effect = DROPEFFECT_NONE;
        let _ = DoDragDrop(&data, &src, DROPEFFECT_COPY | DROPEFFECT_MOVE, &mut effect);
        let _ = Interface::as_raw(&data);
        effect
    }
}

// ----------------------------------------------------------------- wndproc

pub(super) fn lo(l: LPARAM) -> i32 {
    (l.0 & 0xFFFF) as i16 as i32
}
pub(super) fn hi(l: LPARAM) -> i32 {
    ((l.0 >> 16) & 0xFFFF) as i16 as i32
}
