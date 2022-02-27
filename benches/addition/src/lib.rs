#[no_mangle]
extern "C" fn ffi_addition(a: u32, b: u32) -> u32 { 
    a + b
}
