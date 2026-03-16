use shiguredo_webrtc::AdaptedVideoTrackSource;

pub(crate) fn push_i420_frame(
    source: &mut AdaptedVideoTrackSource,
    frame: &crate::VideoFrame,
) -> crate::Result<()> {
    if frame.format != crate::video::VideoFormat::I420 {
        return Err(crate::Error::new(format!(
            "unsupported video format: expected I420, got {}",
            frame.format
        )));
    }

    let size = frame
        .size()
        .ok_or_else(|| crate::Error::new("video frame size is required"))?;
    let width = size.width;
    let height = size.height;

    let uv_width = width.div_ceil(2);
    let uv_height = height.div_ceil(2);
    let y_size = width * height;
    let uv_size = uv_width * uv_height;
    if frame.data.len() < y_size + uv_size * 2 {
        return Err(crate::Error::new("insufficient I420 data"));
    }

    let (y_plane, rest) = frame.data.split_at(y_size);
    let (u_plane, v_plane) = rest.split_at(uv_size);

    let buffer = shiguredo_webrtc::I420Buffer::new(width as i32, height as i32);
    unsafe {
        copy_plane(
            buffer.y_data().as_ptr() as *mut u8,
            buffer.stride_y() as usize,
            y_plane,
            width,
            height,
        );
        copy_plane(
            buffer.u_data().as_ptr() as *mut u8,
            buffer.stride_u() as usize,
            u_plane,
            uv_width,
            uv_height,
        );
        copy_plane(
            buffer.v_data().as_ptr() as *mut u8,
            buffer.stride_v() as usize,
            v_plane,
            uv_width,
            uv_height,
        );
    }

    let timestamp_us = i64::try_from(frame.timestamp.as_micros()).unwrap_or(i64::MAX);
    let webrtc_frame = shiguredo_webrtc::VideoFrame::from_i420(&buffer, timestamp_us, 0);
    source.on_frame(&webrtc_frame);
    Ok(())
}

unsafe fn copy_plane(dst: *mut u8, dst_stride: usize, src: &[u8], width: usize, height: usize) {
    for row in 0..height {
        let src_offset = row * width;
        let dst_offset = row * dst_stride;
        unsafe {
            let src_ptr = src.as_ptr().add(src_offset);
            let dst_ptr = dst.add(dst_offset);
            std::ptr::copy_nonoverlapping(src_ptr, dst_ptr, width);
        }
    }
}
