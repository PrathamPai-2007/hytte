//! Windows Terminal tab targeting.
//! A window focus can't pick a tab, so when a task is first seen we ask UI
//! Automation which tab of its terminal window is selected (the user has just
//! pressed Enter there) and remember that tab element. Clicking the task later
//! selects it through SelectionItemPattern. Best effort: non-WT terminals,
//! closed tabs and UIA failures just fall back to plain window focus.
//! Everything runs on one worker thread, so slow cross-process UIA calls never
//! touch the UI thread.

use crossbeam_channel::{unbounded, Sender};
use std::collections::HashMap;
use windows::core::Interface;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::WindowsAndMessaging::GetClassNameW;

pub enum Cmd {
    /// Remember the selected tab of the terminal that owns `pid`, keyed by task id.
    Snap(String, u32),
    /// Switch to the tab remembered for the task id.
    Select(String),
}

const MAX_REMEMBERED: usize = 128;

pub fn spawn() -> Sender<Cmd> {
    let (tx, rx) = unbounded::<Cmd>();
    std::thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let Ok(uia) = (unsafe {
            CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
        }) else {
            return;
        };
        let mut tabs: HashMap<String, IUIAutomationElement> = HashMap::new();
        while let Ok(c) = rx.recv() {
            match c {
                Cmd::Snap(id, pid) => {
                    if let Some(el) = selected_tab(&uia, pid) {
                        if tabs.len() >= MAX_REMEMBERED {
                            tabs.clear();
                        }
                        tabs.insert(id, el);
                    }
                }
                Cmd::Select(id) => {
                    if let Some(el) = tabs.get(&id) {
                        select(el);
                    }
                }
            }
        }
    });
    tx
}

fn is_terminal_window(h: windows::Win32::Foundation::HWND) -> bool {
    let mut cls = [0u16; 64];
    let n = unsafe { GetClassNameW(h, &mut cls) } as usize;
    String::from_utf16_lossy(&cls[..n]) == "CASCADIA_HOSTING_WINDOW_CLASS"
}

fn selected_tab(uia: &IUIAutomation, pid: u32) -> Option<IUIAutomationElement> {
    let hwnd = crate::proc::terminal_window_for(pid).filter(|h| is_terminal_window(*h))?;
    unsafe {
        let root = uia.ElementFromHandle(hwnd).ok()?;
        let cond = uia
            .CreatePropertyCondition(
                UIA_ControlTypePropertyId,
                &VARIANT::from(UIA_TabItemControlTypeId.0),
            )
            .ok()?;
        let all = root.FindAll(TreeScope_Descendants, &cond).ok()?;
        for i in 0..all.Length().ok()? {
            let el = all.GetElement(i).ok()?;
            let sel = el
                .GetCurrentPattern(UIA_SelectionItemPatternId)
                .and_then(|p| p.cast::<IUIAutomationSelectionItemPattern>())
                .and_then(|p| p.CurrentIsSelected())
                .map(|b| b.as_bool())
                .unwrap_or(false);
            if sel {
                return Some(el);
            }
        }
        None
    }
}

fn select(el: &IUIAutomationElement) {
    unsafe {
        if let Ok(p) = el
            .GetCurrentPattern(UIA_SelectionItemPatternId)
            .and_then(|p| p.cast::<IUIAutomationSelectionItemPattern>())
        {
            let _ = p.Select();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Manual: needs a Windows Terminal window with 2+ tabs; HYTTE_WT_PID is any shell pid inside it.
    #[test]
    #[ignore]
    fn selects_other_tab() {
        let pid: u32 = std::env::var("HYTTE_WT_PID").unwrap().parse().unwrap();
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let uia: IUIAutomation =
            unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).unwrap() };
        let name = |e: &IUIAutomationElement| unsafe { e.CurrentName().unwrap().to_string() };
        let before = selected_tab(&uia, pid).expect("selected tab found");
        println!("selected before: {}", name(&before));
        let hwnd = crate::proc::terminal_window_for(pid).unwrap();
        unsafe {
            let root = uia.ElementFromHandle(hwnd).unwrap();
            let cond = uia
                .CreatePropertyCondition(
                    UIA_ControlTypePropertyId,
                    &VARIANT::from(UIA_TabItemControlTypeId.0),
                )
                .unwrap();
            let all = root.FindAll(TreeScope_Descendants, &cond).unwrap();
            let other = (0..all.Length().unwrap())
                .map(|i| all.GetElement(i).unwrap())
                .find(|e| name(e) != name(&before))
                .expect("a second tab");
            select(&other);
            std::thread::sleep(std::time::Duration::from_millis(500));
            let after = selected_tab(&uia, pid).unwrap();
            println!("selected after: {}", name(&after));
            assert_ne!(name(&after), name(&before));
            select(&before);
        }
    }
}
