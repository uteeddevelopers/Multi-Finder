//! Image text via `Windows.Media.Ocr` (Windows + feature `ocr`). NOT compiled on Linux.
//!
//! Runs on its own short-lived thread so the COM/WinRT apartment it initializes (MTA) never
//! clashes with other COM users (the `trash` crate initializes STA on its threads).

use std::path::{Path, PathBuf};

use windows::core::{Result as WinResult, HSTRING};
use windows::Graphics::Imaging::{
    BitmapAlphaMode, BitmapDecoder, BitmapInterpolationMode, BitmapPixelFormat, BitmapTransform,
    ColorManagementMode, ExifOrientationMode,
};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::{FileAccessMode, StorageFile};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

/// Recognized text of an image (user-profile languages), None when unreadable or empty.
pub(crate) fn ocr_image(path: &Path) -> Option<String> {
    let path: PathBuf = path.to_path_buf();
    std::thread::Builder::new()
        .name("mf-ocr".into())
        .spawn(move || {
            // SAFETY: plain COM initialization for this new thread, balanced below.
            let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            let result = recognize(&path).ok().flatten();
            if hr.is_ok() {
                // SAFETY: matches the successful CoInitializeEx above.
                unsafe { CoUninitialize() };
            }
            result
        })
        .ok()?
        .join()
        .ok()
        .flatten()
}

fn recognize(path: &Path) -> WinResult<Option<String>> {
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path))?.join()?;
    let stream = file.OpenAsync(FileAccessMode::Read)?.join()?;
    let decoder = BitmapDecoder::CreateAsync(&stream)?.join()?;
    let max = OcrEngine::MaxImageDimension()?;
    let (w, h) = (decoder.PixelWidth()?, decoder.PixelHeight()?);
    let transform = BitmapTransform::new()?;
    if w > max || h > max {
        let scale = f64::from(max) / f64::from(w.max(h));
        transform.SetScaledWidth(((f64::from(w) * scale) as u32).max(1))?;
        transform.SetScaledHeight(((f64::from(h) * scale) as u32).max(1))?;
        transform.SetInterpolationMode(BitmapInterpolationMode::Fant)?;
    }
    let bitmap = decoder
        .GetSoftwareBitmapTransformedAsync(
            BitmapPixelFormat::Bgra8,
            BitmapAlphaMode::Premultiplied,
            &transform,
            ExifOrientationMode::RespectExifOrientation,
            ColorManagementMode::DoNotColorManage,
        )?
        .join()?;
    let engine = OcrEngine::TryCreateFromUserProfileLanguages()?;
    let result = engine.RecognizeAsync(&bitmap)?.join()?;
    let text = result.Text()?.to_string_lossy();
    Ok(if text.trim().is_empty() {
        None
    } else {
        Some(text)
    })
}
