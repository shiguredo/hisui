use crate::{
    Error,
    video::{VideoFormat, VideoFrame},
};

#[derive(Debug)]
pub struct I420Canvas {
    width: usize,
    height: usize,
    data: Vec<u8>,
}

impl I420Canvas {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            data: black_i420_data(width, height),
        }
    }

    pub fn draw_frame_clipped(
        &mut self,
        x: isize,
        y: isize,
        frame: &VideoFrame,
    ) -> crate::Result<()> {
        if frame.format != VideoFormat::I420 {
            return Err(Error::new("unsupported video format: expected I420"));
        }
        let size = frame
            .size()
            .ok_or_else(|| Error::new("video frame size is required"))?;

        let src_y_size = size.width.saturating_mul(size.height);
        let src_uv_width = size.width.div_ceil(2);
        let src_uv_height = size.height.div_ceil(2);
        let src_uv_size = src_uv_width.saturating_mul(src_uv_height);

        if frame.data.len() < src_y_size.saturating_add(src_uv_size.saturating_mul(2)) {
            return Err(Error::new("invalid I420 frame size"));
        }

        let src_y = &frame.data[..src_y_size];
        let src_u = &frame.data[src_y_size..][..src_uv_size];
        let src_v = &frame.data[src_y_size + src_uv_size..][..src_uv_size];

        let (src_x0, dst_x0, copy_width) = clipped_span(size.width, self.width, x);
        let (src_y0, dst_y0, copy_height) = clipped_span(size.height, self.height, y);

        if copy_width == 0 || copy_height == 0 {
            return Ok(());
        }

        for row in 0..copy_height {
            let src_offset = (src_y0 + row) * size.width + src_x0;
            let dst_offset = (dst_y0 + row) * self.width + dst_x0;
            self.data[dst_offset..][..copy_width]
                .copy_from_slice(&src_y[src_offset..][..copy_width]);
        }

        let dst_y_size = self.width.saturating_mul(self.height);
        let dst_uv_width = self.width.div_ceil(2);
        let dst_uv_height = self.height.div_ceil(2);
        let dst_uv_size = dst_uv_width.saturating_mul(dst_uv_height);

        let src_uv_x0 = src_x0 / 2;
        let src_uv_y0 = src_y0 / 2;
        let dst_uv_x0 = dst_x0 / 2;
        let dst_uv_y0 = dst_y0 / 2;
        let copy_uv_width = copy_width.div_ceil(2);
        let copy_uv_height = copy_height.div_ceil(2);

        for row in 0..copy_uv_height {
            let src_offset = (src_uv_y0 + row) * src_uv_width + src_uv_x0;
            let dst_offset = (dst_uv_y0 + row) * dst_uv_width + dst_uv_x0;

            let dst_u_offset = dst_y_size + dst_offset;
            let dst_v_offset = dst_y_size + dst_uv_size + dst_offset;

            self.data[dst_u_offset..][..copy_uv_width]
                .copy_from_slice(&src_u[src_offset..][..copy_uv_width]);
            self.data[dst_v_offset..][..copy_uv_width]
                .copy_from_slice(&src_v[src_offset..][..copy_uv_width]);
        }

        Ok(())
    }

    pub fn into_data(self) -> Vec<u8> {
        self.data
    }
}

fn clipped_span(src_len: usize, dst_len: usize, dst_pos: isize) -> (usize, usize, usize) {
    let dst_start = dst_pos.max(0) as usize;
    let src_start = if dst_pos < 0 {
        dst_pos.unsigned_abs()
    } else {
        0
    };

    let src_remaining = src_len.saturating_sub(src_start);
    let dst_remaining = dst_len.saturating_sub(dst_start);
    let copy_len = src_remaining.min(dst_remaining);

    (src_start, dst_start, copy_len)
}

fn black_i420_data(width: usize, height: usize) -> Vec<u8> {
    let y_size = width.saturating_mul(height);
    let uv_size = width.div_ceil(2).saturating_mul(height.div_ceil(2));
    let mut data = vec![0; y_size.saturating_add(uv_size.saturating_mul(2))];
    data[y_size..].fill(128);
    data
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::I420Canvas;
    use crate::{
        types::EvenUsize,
        video::{VideoFormat, VideoFrame},
    };

    #[test]
    fn draw_frame_clipped_with_negative_position() -> crate::Result<()> {
        let mut canvas = I420Canvas::new(8, 8);
        let expected = I420Canvas::new(8, 8).into_data();
        let frame = patterned_i420_frame();

        canvas.draw_frame_clipped(-2, -2, &frame)?;

        assert_ne!(canvas.into_data(), expected);
        Ok(())
    }

    #[test]
    fn draw_frame_clipped_rejects_non_i420() {
        let mut canvas = I420Canvas::new(8, 8);
        let mut frame = patterned_i420_frame();
        frame.format = VideoFormat::H264;

        let result = canvas.draw_frame_clipped(0, 0, &frame);
        assert!(result.is_err());
    }

    #[test]
    fn draw_frame_clipped_rejects_invalid_i420_size() {
        let mut canvas = I420Canvas::new(8, 8);
        let mut frame = patterned_i420_frame();
        frame.data.truncate(frame.data.len().saturating_sub(1));

        let result = canvas.draw_frame_clipped(0, 0, &frame);
        assert!(result.is_err());
    }

    #[test]
    fn draw_frame_clipped_fully_outside_keeps_background() -> crate::Result<()> {
        let mut canvas = I420Canvas::new(8, 8);
        let expected = I420Canvas::new(8, 8).into_data();
        let frame = patterned_i420_frame();

        canvas.draw_frame_clipped(16, 16, &frame)?;

        assert_eq!(canvas.into_data(), expected);
        Ok(())
    }

    fn patterned_i420_frame() -> VideoFrame {
        let mut frame = VideoFrame::black(
            EvenUsize::new(4).expect("infallible"),
            EvenUsize::new(4).expect("infallible"),
        );
        let size = frame.size().expect("infallible");
        let y_size = size.width * size.height;
        let uv_size = size.width.div_ceil(2) * size.height.div_ceil(2);

        frame.data[..y_size].fill(200);
        frame.data[y_size..][..uv_size].fill(64);
        frame.data[y_size + uv_size..][..uv_size].fill(192);
        frame.timestamp = Duration::from_millis(10);
        frame
    }
}
