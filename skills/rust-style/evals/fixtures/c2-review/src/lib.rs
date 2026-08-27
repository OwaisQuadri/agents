#[allow(clippy::missing_safety_doc)]
pub unsafe fn first_unchecked(values: &[u8]) -> u8 {
    unsafe { *values.as_ptr() }
}
