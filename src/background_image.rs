use crate::config::BackgroundMode;

pub fn load_image(path: &str) -> cairo::ImageSurface {
    let image = match image::open(&path) {
        Ok(i) => i,
        Err(e) => {
            panic!("Failed to open image {path} with error {e:?}")
        }
    };

    let image = image.to_rgba8();

    let mut cairo_surface = cairo::ImageSurface::create(
        cairo::Format::ARgb32,
        image.width() as i32,
        image.height() as i32,
    )
    .expect("Failed to create Cairo surface");

    {
        let mut cairo_surface_data = cairo_surface.data();
        for (pixel, argb) in image
            .pixels()
            .zip(cairo_surface_data.as_mut().unwrap().chunks_exact_mut(4))
        {
            // There might be a better way to do this, but since we are doing this
            // one-off the performance seems okay.
            argb[3] = pixel.0[3];
            argb[2] = pixel.0[0];
            argb[1] = pixel.0[1];
            argb[0] = pixel.0[2];
        }
    }

    cairo_surface
}

pub fn render_background_image(
    context: &cairo::Context,
    image: &cairo::ImageSurface,
    mode: BackgroundMode,
    buffer_width: i32,
    buffer_height: i32,
) {
    let (width, height) = (image.width(), image.height());

    let window_ratio = (buffer_width as f64) / (buffer_height as f64);
    let bg_ratio = (width as f64) / (height as f64);
    let width_ratio = (buffer_width as f64) / (width as f64);
    let height_ratio = (buffer_height as f64) / (height as f64);

    context.save().unwrap();

    match mode {
        BackgroundMode::Stretch => {
            context.scale(width_ratio, height_ratio);
            context.set_source_surface(&image, 0.0, 0.0).unwrap();
        }
        BackgroundMode::Fill | BackgroundMode::Fit => {
            let (scale, offset_x, offset_y) = {
                if (mode == BackgroundMode::Fill && window_ratio > bg_ratio)
                    || (mode == BackgroundMode::Fit && window_ratio < bg_ratio)
                {
                    let scale = width_ratio;
                    let offset = (buffer_height as f64) / 2.0 / scale - (height as f64) / 2.0;
                    (scale, 0.0, offset)
                } else {
                    let scale = height_ratio;
                    let offset = (buffer_width as f64) / 2.0 / scale - (width as f64) / 2.0;
                    (scale, offset, 0.0)
                }
            };
            context.scale(scale, scale);
            context
                .set_source_surface(&image, offset_x, offset_y)
                .unwrap();
        }
        BackgroundMode::Center => {
            let offset_x = (buffer_width as f64) / 2.0 - (width as f64) / 2.0;
            let offset_y = (buffer_height as f64) / 2.0 - (height as f64) / 2.0;
            context
                .set_source_surface(&image, offset_x, offset_y)
                .unwrap();
        }
        BackgroundMode::Tile => {
            let pattern = cairo::SurfacePattern::create(image);
            pattern.set_extend(cairo::Extend::Repeat);
            context.set_source(pattern).unwrap();
        }
        BackgroundMode::SolidColor => {}
    };
    context.paint().unwrap();
    context.restore().unwrap();
}
