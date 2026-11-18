#[cfg(target_os = "macos")]
pub fn change_count() -> Option<u64> {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};

    let class = AnyClass::get(c"NSPasteboard")?;
    // SAFETY: +generalPasteboard takes no arguments and returns the shared pasteboard or nil.
    let pasteboard: *mut AnyObject = unsafe { msg_send![class, generalPasteboard] };
    if pasteboard.is_null() {
        return None;
    }
    // SAFETY: the pointer was just checked and -changeCount returns an NSInteger.
    let count: isize = unsafe { msg_send![pasteboard, changeCount] };
    Some(count as u64)
}

#[cfg(windows)]
pub fn change_count() -> Option<u64> {
    // SAFETY: the call takes no arguments and only reads a counter of the window station.
    let count = unsafe { windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber() };
    (count != 0).then_some(count as u64)
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn change_count() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn reading_the_counter_never_panics_and_does_not_go_backwards() {
        let first = super::change_count();
        let second = super::change_count();
        if let (Some(a), Some(b)) = (first, second) {
            assert!(b >= a);
        }
    }
}
