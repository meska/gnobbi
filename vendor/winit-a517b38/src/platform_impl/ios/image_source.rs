//! ImageIO metadata preflight before UIKit is allowed to decode the raster.
use std::ffi::c_void;

use core_foundation::base::{CFRelease, CFTypeRef};
use core_foundation::dictionary::CFDictionaryGetValue;
use core_foundation::number::{CFNumberGetValue, kCFNumberDoubleType};
use core_foundation::string::CFStringRef;
use objc2_foundation::NSData;

#[link(name = "ImageIO", kind = "framework")]
extern "C" {
    fn CGImageSourceCreateWithData(data: CFTypeRef, options: CFTypeRef) -> CFTypeRef;
    fn CGImageSourceGetPrimaryImageIndex(source: CFTypeRef) -> usize;
    fn CGImageSourceCopyPropertiesAtIndex(source: CFTypeRef, index: usize, options: CFTypeRef) -> CFTypeRef;
    static kCGImagePropertyPixelWidth: CFStringRef;
    static kCGImagePropertyPixelHeight: CFStringRef;
}

pub(super) fn dimensions(data: &NSData) -> Option<(f64, f64)> {
    unsafe {
        // NSData and CFData are toll-free bridged. No decoded pixels are requested here.
        let source = CGImageSourceCreateWithData(data as *const NSData as CFTypeRef, std::ptr::null());
        if source.is_null() { return None; }
        // HEIF/adaptive sources can place their primary image after thumbnail representations.
        let primary = CGImageSourceGetPrimaryImageIndex(source);
        let properties = CGImageSourceCopyPropertiesAtIndex(source, primary, std::ptr::null());
        let dimensions = if properties.is_null() {
            None
        } else {
            let width = CFDictionaryGetValue(properties.cast(), kCGImagePropertyPixelWidth.cast());
            let height = CFDictionaryGetValue(properties.cast(), kCGImagePropertyPixelHeight.cast());
            let (mut w, mut h) = (0.0f64, 0.0f64);
            let valid = !width.is_null() && !height.is_null()
                && CFNumberGetValue(width.cast(), kCFNumberDoubleType, &mut w as *mut f64 as *mut c_void)
                && CFNumberGetValue(height.cast(), kCFNumberDoubleType, &mut h as *mut f64 as *mut c_void);
            CFRelease(properties);
            valid.then_some((w, h))
        };
        CFRelease(source);
        dimensions
    }
}
