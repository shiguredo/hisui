use crate::TrackId;

#[derive(Debug)]
pub struct VideoMixer2 {
    pub canvas_width: usize,
    pub canvas_height: usize,
    pub input_tracks: Vec<InputTrack>,
    pub output_track_id: TrackId,
}

impl nojson::DisplayJson for VideoMixer2 {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("canvasWidth", self.canvas_width)?;
            f.member("canvasHeight", self.canvas_height)?;
            f.member("inputTracks", &self.input_tracks)?;
            f.member("outputTrackId", &self.output_track_id)
        })
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for VideoMixer2 {
    type Error = nojson::JsonParseError;

    fn try_from(
        value: nojson::RawJsonValue<'text, 'raw>,
    ) -> std::result::Result<Self, Self::Error> {
        let canvas_width: usize = value.to_member("canvasWidth")?.required()?.try_into()?;
        let canvas_height: usize = value.to_member("canvasHeight")?.required()?.try_into()?;
        let input_tracks: Vec<InputTrack> =
            value.to_member("inputTracks")?.required()?.try_into()?;
        let output_track_id: TrackId = value.to_member("outputTrackId")?.required()?.try_into()?;

        Ok(Self {
            canvas_width,
            canvas_height,
            input_tracks,
            output_track_id,
        })
    }
}

#[derive(Debug)]
pub struct InputTrack {
    pub track_id: TrackId,
    pub x: isize,
    pub y: isize,
    pub width: usize,
    pub height: usize,
}

impl nojson::DisplayJson for InputTrack {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        f.object(|f| {
            f.member("trackId", &self.track_id)?;
            f.member("x", self.x)?;
            f.member("y", self.y)?;
            f.member("width", self.width)?;
            f.member("height", self.height)
        })
    }
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for InputTrack {
    type Error = nojson::JsonParseError;

    fn try_from(
        value: nojson::RawJsonValue<'text, 'raw>,
    ) -> std::result::Result<Self, Self::Error> {
        let track_id: TrackId = value.to_member("trackId")?.required()?.try_into()?;
        let x: isize = value.to_member("x")?.required()?.try_into()?;
        let y: isize = value.to_member("y")?.required()?.try_into()?;
        let width: usize = value.to_member("width")?.required()?.try_into()?;
        let height: usize = value.to_member("height")?.required()?.try_into()?;

        Ok(Self {
            track_id,
            x,
            y,
            width,
            height,
        })
    }
}
