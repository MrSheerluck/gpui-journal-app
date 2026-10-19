//! Video frames: decode on a worker thread with `AVAssetReader`, pace by
//! presentation time, and hand frames to the UI over a bounded channel.
//! Sound for the same file rides `media::Player` (`AVAudioPlayer` plays a
//! video container's audio track unchanged), so this module is frames only.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// One decoded frame, top-down **BGRA**, tightly packed — the order
/// `RenderImage` wants — with its position on the asset's timeline.
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
    /// Presentation time from the start of the stream.
    pub pts_ms: u64,
    /// The stream's total length, when the track reports one.
    pub duration_ms: Option<u64>,
}

impl VideoFrame {
    /// The frame as a renderer-ready image.
    pub fn into_render_image(self) -> Option<Arc<gpui_kit::RenderImage>> {
        let buffer = image::RgbaImage::from_raw(self.width, self.height, self.bgra)?;
        Some(Arc::new(gpui_kit::RenderImage::new([image::Frame::new(
            buffer,
        )])))
    }
}

/// A running decode of one video. Dropping it stops the worker.
pub struct Pump {
    frames: async_channel::Receiver<Result<VideoFrame, String>>,
    control: Arc<Control>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Pump {
    /// Starts decoding `path` from the beginning at up to 640 px wide.
    pub fn start(path: &Path) -> Result<Self, String> {
        let (sender, frames) = async_channel::bounded(2);
        let control = Arc::new(Control::default());
        let worker = platform::spawn(path.to_path_buf(), sender, control.clone())?;
        Ok(Self {
            frames,
            control,
            worker: Some(worker),
        })
    }

    /// Decodes just the first frame of a video, for a card's poster. The
    /// decoder is dropped as soon as that frame arrives.
    pub async fn first_frame(path: &std::path::Path) -> Result<VideoFrame, String> {
        let pump = Pump::start(path)?;
        let frames = pump.frames();
        let frame = frames
            .recv()
            .await
            .map_err(|_| "the decoder stopped before its first frame".to_string())?;
        drop(pump);
        frame
    }

    /// Frames and setup failures, in order.
    pub fn frames(&self) -> async_channel::Receiver<Result<VideoFrame, String>> {
        self.frames.clone()
    }

    /// Whether the decode has ended (end of stream or a failure).
    pub fn finished(&self) -> bool {
        self.control.stopped()
    }

    pub fn pause(&self) {
        self.control.paused.store(true, Ordering::Relaxed);
    }

    pub fn resume(&self) {
        self.control.paused.store(false, Ordering::Relaxed);
    }
}

impl Drop for Pump {
    fn drop(&mut self) {
        self.control.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Shared worker control: pause and stop are checked between frames.
#[derive(Default)]
pub(crate) struct Control {
    paused: AtomicBool,
    stop: AtomicBool,
}

impl Control {
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }
}

/// Hands a frame to the UI without blocking indefinitely: the channel is
/// bounded, so a slow consumer applies backpressure this loop can interrupt.
fn send_frame(
    sender: &async_channel::Sender<Result<VideoFrame, String>>,
    frame: VideoFrame,
    control: &Control,
) -> bool {
    let mut pending = Ok(frame);
    loop {
        if control.stopped() {
            return false;
        }
        match sender.try_send(pending) {
            Ok(()) => return true,
            Err(async_channel::TrySendError::Full(frame)) => {
                pending = frame;
                std::thread::sleep(std::time::Duration::from_millis(8));
            }
            Err(async_channel::TrySendError::Closed(_)) => return false,
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use objc2::rc::Retained;
    use objc2_av_foundation::{
        AVAssetReader, AVAssetReaderTrackOutput, AVAssetTrack, AVMediaTypeVideo, AVURLAsset,
    };
    use objc2_core_video::{
        CVPixelBuffer, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow,
        CVPixelBufferGetHeight, CVPixelBufferGetPixelFormatType, CVPixelBufferGetWidth,
        CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
        kCVPixelBufferHeightKey, kCVPixelBufferPixelFormatTypeKey, kCVPixelBufferWidthKey,
        kCVPixelFormatType_32BGRA,
    };
    use objc2_foundation::{NSDictionary, NSNumber, NSString, NSUInteger, NSURL};

    use super::{Control, VideoFrame, send_frame};

    /// The longest edge worth decoding for an entry card.
    const MAX_WIDTH: f32 = 640.;

    pub fn spawn(
        path: PathBuf,
        sender: async_channel::Sender<Result<VideoFrame, String>>,
        control: Arc<Control>,
    ) -> Result<std::thread::JoinHandle<()>, String> {
        std::thread::Builder::new()
            .name("dainiki-video".into())
            .spawn(move || {
                let result = decode(&path, &sender, &control);
                // The decode is over either way; `finished` reports it so a
                // replay starts a fresh pump.
                control
                    .stop
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                if let Err(error) = result {
                    // A closed channel means the pump is gone: nothing to report.
                    sender.send_blocking(Err(error)).ok();
                }
            })
            .map_err(|error| format!("could not start the video worker — {error}"))
    }

    fn file_url(path: &std::path::Path) -> Result<Retained<NSURL>, String> {
        let text = path
            .to_str()
            .ok_or_else(|| "the file path is not valid UTF-8".to_string())?;
        Ok(NSURL::fileURLWithPath(&NSString::from_str(text)))
    }

    fn describe(error: &objc2_foundation::NSError) -> String {
        error.localizedDescription().to_string()
    }

    /// CFString constants are toll-free bridged to NSString.
    fn bridged<T>(key: &'static T) -> &'static NSString {
        // SAFETY: CoreVideo key constants are CFStrings; CFString and
        // NSString share their representation.
        unsafe { &*(key as *const T as *const NSString) }
    }

    /// The frame size to decode: at most 640 px wide, even dimensions.
    fn target_size(track: &AVAssetTrack) -> (u32, u32) {
        let natural = unsafe { track.naturalSize() };
        let width = natural.width as f32;
        let height = natural.height as f32;
        if width <= 0. || height <= 0. {
            return (640, 360);
        }
        let scale = (MAX_WIDTH / width).min(1.0);
        let even = |value: f32| ((value.round() as u32).max(2)) & !1;
        (even(width * scale), even(height * scale))
    }

    fn output_settings(width: u32, height: u32) -> Retained<NSDictionary<NSString, NSNumber>> {
        let format = NSNumber::numberWithUnsignedInt(kCVPixelFormatType_32BGRA);
        let width = NSNumber::numberWithUnsignedInteger(width as NSUInteger);
        let height = NSNumber::numberWithUnsignedInteger(height as NSUInteger);
        NSDictionary::from_slices(
            &[
                bridged(unsafe { kCVPixelBufferPixelFormatTypeKey }),
                bridged(unsafe { kCVPixelBufferWidthKey }),
                bridged(unsafe { kCVPixelBufferHeightKey }),
            ],
            &[&format, &width, &height],
        )
    }

    fn decode(
        path: &std::path::Path,
        sender: &async_channel::Sender<Result<VideoFrame, String>>,
        control: &Control,
    ) -> Result<(), String> {
        let url = file_url(path)?;
        let asset = unsafe { AVURLAsset::URLAssetWithURL_options(&url, None) };
        let media_type =
            unsafe { AVMediaTypeVideo }.ok_or("the video media type is unavailable")?;
        #[allow(deprecated)] // The async track loader is not in the pinned bindings.
        let tracks = unsafe { asset.tracksWithMediaType(media_type) };
        let track = tracks
            .firstObject()
            .ok_or_else(|| "the file has no video track".to_string())?;

        let (width, height) = target_size(&track);
        let duration_ms = {
            #[allow(deprecated)] // The sync reader is the pinned bindings' shape.
            let range = unsafe { track.timeRange() };
            let seconds = unsafe { range.duration.seconds() };
            (seconds > 0.0).then_some((seconds * 1000.0) as u64)
        };
        let settings = output_settings(width, height);
        // The bindings ask for `AnyObject` values; the dictionary stores
        // NSNumbers either way (the generic parameter is a type-level tag).
        let settings_any: &NSDictionary<NSString, objc2::runtime::AnyObject> =
            unsafe { &*(Retained::as_ptr(&settings) as *const _) };
        let output = unsafe {
            AVAssetReaderTrackOutput::assetReaderTrackOutputWithTrack_outputSettings(
                &track,
                Some(settings_any),
            )
        };
        let reader = unsafe { AVAssetReader::assetReaderWithAsset_error(&asset) }
            .map_err(|e| describe(&e))?;
        if !unsafe { reader.canAddOutput(&output) } {
            return Err("the video track cannot be read".into());
        }
        unsafe { reader.addOutput(&output) };
        if !unsafe { reader.startReading() } {
            return Err("the video could not start decoding".into());
        }

        let mut play_started = Instant::now();
        let mut paused_at: Option<Instant> = None;
        let mut first_pts: Option<f64> = None;
        loop {
            if control.stopped() {
                return Ok(());
            }
            if control.is_paused() {
                paused_at.get_or_insert_with(Instant::now);
                std::thread::sleep(Duration::from_millis(15));
                continue;
            }
            if let Some(at) = paused_at.take() {
                play_started += at.elapsed();
            }
            let Some(sample) = (unsafe { output.copyNextSampleBuffer() }) else {
                return Ok(());
            };
            if !unsafe { sample.is_valid() } {
                continue;
            }
            let seconds = unsafe { sample.presentation_time_stamp().seconds() };
            let start = *first_pts.get_or_insert(seconds);
            let target = Duration::from_secs_f64((seconds - start).max(0.0));
            let pts_ms = target.as_millis() as u64;
            while play_started.elapsed() < target {
                if control.stopped() {
                    return Ok(());
                }
                if control.is_paused() {
                    break;
                }
                let remaining = target.saturating_sub(play_started.elapsed());
                std::thread::sleep(remaining.min(Duration::from_millis(10)));
            }
            let Some(buffer) = (unsafe { sample.image_buffer() }) else {
                continue;
            };
            let frame = convert(&buffer, pts_ms, duration_ms)?;
            if !send_frame(sender, frame, control) {
                return Ok(());
            }
        }
    }

    /// Copies a locked BGRA pixel buffer into tightly packed RGBA.
    fn convert(
        buffer: &objc2_core_video::CVImageBuffer,
        pts_ms: u64,
        duration_ms: Option<u64>,
    ) -> Result<VideoFrame, String> {
        let pixel_buffer: &CVPixelBuffer = buffer;
        let format = CVPixelBufferGetPixelFormatType(pixel_buffer);
        if format != kCVPixelFormatType_32BGRA {
            return Err(format!("unsupported pixel format {format:#x}"));
        }
        let width = CVPixelBufferGetWidth(pixel_buffer);
        let height = CVPixelBufferGetHeight(pixel_buffer);
        if width == 0 || height == 0 {
            return Err("the video produced an empty frame".into());
        }
        let lock =
            unsafe { CVPixelBufferLockBaseAddress(pixel_buffer, CVPixelBufferLockFlags::ReadOnly) };
        if lock != 0 {
            return Err(format!("could not lock a video frame ({lock})"));
        }
        let base = CVPixelBufferGetBaseAddress(pixel_buffer) as *const u8;
        let stride = CVPixelBufferGetBytesPerRow(pixel_buffer);
        let mut bgra = vec![0u8; width * height * 4];
        if !base.is_null() {
            // SAFETY: the buffer is locked for reading, `stride` covers the
            // row, and `width * 4` bytes fit inside it.
            unsafe {
                for row in 0..height {
                    let source = base.add(row * stride);
                    let destination = bgra.as_mut_ptr().add(row * width * 4);
                    std::ptr::copy_nonoverlapping(source, destination, width * 4);
                }
            }
        }
        unsafe {
            CVPixelBufferUnlockBaseAddress(pixel_buffer, CVPixelBufferLockFlags::ReadOnly);
        }
        // CoreVideo hands back BGRA already, which is exactly what the
        // renderer wants; swapping here used to tint every frame.
        Ok(VideoFrame {
            width: width as u32,
            height: height as u32,
            bgra,
            pts_ms,
            duration_ms,
        })
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use std::path::PathBuf;
    use std::sync::Arc;

    use super::{Control, VideoFrame};

    pub fn spawn(
        _path: PathBuf,
        _sender: async_channel::Sender<Result<VideoFrame, String>>,
        _control: Arc<Control>,
    ) -> Result<std::thread::JoinHandle<()>, String> {
        Err("video playback is not implemented on this platform yet".into())
    }
}

#[cfg(test)]
mod tests {
    /// Manual verification against a real file:
    /// `DAINIKI_VIDEO_FIXTURE=/path/clip.mp4 cargo test -p dainiki-desktop
    ///  decodes_frames -- --ignored --nocapture`
    #[test]
    #[ignore = "manual: needs DAINIKI_VIDEO_FIXTURE"]
    fn decodes_frames_from_a_local_video() {
        let path = std::env::var("DAINIKI_VIDEO_FIXTURE").expect("DAINIKI_VIDEO_FIXTURE");
        let pump = super::Pump::start(std::path::Path::new(&path)).expect("start the pump");
        let frames = pump.frames();
        let frame = frames.recv_blocking().expect("a frame").expect("decoded");
        assert!(frame.width > 0 && frame.height > 0);
        assert_eq!(
            frame.bgra.len(),
            (frame.width * frame.height * 4) as usize,
            "frames are tightly packed RGBA"
        );
        assert!(
            frame.bgra.iter().any(|byte| *byte != 0),
            "the frame is not blank"
        );
        println!("decoded {}x{}", frame.width, frame.height);
    }
}
