use std::path::Path;

use magnus::{Error, RString, Ruby, class, define_module, function, method, prelude::*};

fn html_to_pdf(ruby: &Ruby, html: String) -> Result<RString, Error> {
    let pdf = ironpress_core::html_to_pdf(&html)
        .map_err(|error| Error::new(ruby.exception_runtime_error(), error.to_string()))?;
    Ok(ruby.str_from_slice(&pdf))
}

fn markdown_to_pdf(ruby: &Ruby, markdown: String) -> Result<RString, Error> {
    let pdf = ironpress_core::markdown_to_pdf(&markdown)
        .map_err(|error| Error::new(ruby.exception_runtime_error(), error.to_string()))?;
    Ok(ruby.str_from_slice(&pdf))
}

#[magnus::wrap(class = "Ironpress::NativeConverter", free_immediately, size)]
struct NativeConverter {
    converter: ironpress_core::HtmlConverter,
}

impl NativeConverter {
    fn new() -> Self {
        Self {
            converter: ironpress_core::HtmlConverter::new(),
        }
    }

    fn configured(
        &self,
        configure: impl FnOnce(ironpress_core::HtmlConverter) -> ironpress_core::HtmlConverter,
    ) -> Self {
        Self {
            converter: configure(self.converter.clone()),
        }
    }

    fn page_size(ruby: &Ruby, this: &Self, name: String) -> Result<Self, Error> {
        let size = match name.to_ascii_lowercase().as_str() {
            "a4" => ironpress_core::PageSize::A4,
            "letter" => ironpress_core::PageSize::LETTER,
            "legal" => ironpress_core::PageSize::LEGAL,
            _ => {
                return Err(Error::new(
                    ruby.exception_arg_error(),
                    format!("unknown page size `{name}`"),
                ));
            }
        };
        Ok(this.configured(|converter| converter.page_size(size)))
    }

    fn page_size_custom(&self, width: f64, height: f64) -> Self {
        self.configured(|converter| {
            converter.page_size(ironpress_core::PageSize::new(width as f32, height as f32))
        })
    }

    fn margin(&self, points: f64) -> Self {
        self.configured(|converter| {
            converter.margin(ironpress_core::Margin::uniform(points as f32))
        })
    }

    fn margin_sides(&self, top: f64, right: f64, bottom: f64, left: f64) -> Self {
        self.configured(|converter| {
            converter.margin(ironpress_core::Margin::new(
                top as f32,
                right as f32,
                bottom as f32,
                left as f32,
            ))
        })
    }

    fn compress(&self, enabled: bool) -> Self {
        self.configured(|converter| converter.compress(enabled))
    }

    fn jpeg_quality(&self, quality: u8) -> Self {
        self.configured(|converter| converter.jpeg_quality(quality))
    }

    fn auto_resize_images(&self, enabled: bool) -> Self {
        self.configured(|converter| converter.auto_resize_images(enabled))
    }

    fn image_dpi(&self, dpi: f64) -> Self {
        self.configured(|converter| converter.image_dpi(dpi as f32))
    }

    fn filter_dpi(&self, dpi: f64) -> Self {
        self.configured(|converter| converter.filter_dpi(dpi as f32))
    }

    fn mask_dpi(&self, dpi: f64) -> Self {
        self.configured(|converter| converter.mask_dpi(dpi as f32))
    }

    fn background_raster_dpi(&self, dpi: f64) -> Self {
        self.configured(|converter| converter.background_raster_dpi(dpi as f32))
    }

    fn occlusion_cull(&self, enabled: bool) -> Self {
        self.configured(|converter| converter.occlusion_cull(enabled))
    }

    fn sanitize(&self, enabled: bool) -> Self {
        self.configured(|converter| converter.sanitize(enabled))
    }

    fn accessible(&self, enabled: bool) -> Self {
        self.configured(|converter| converter.accessible(enabled))
    }

    fn document_title(ruby: &Ruby, this: &Self, title: String) -> Result<Self, Error> {
        let title = title
            .parse::<ironpress_core::DocumentTitle>()
            .map_err(|error| Error::new(ruby.exception_arg_error(), error.to_string()))?;
        Ok(this.configured(|converter| converter.document_title(title)))
    }

    fn document_language(ruby: &Ruby, this: &Self, tag: String) -> Result<Self, Error> {
        let language = tag
            .parse::<ironpress_core::DocumentLanguage>()
            .map_err(|error| Error::new(ruby.exception_arg_error(), error.to_string()))?;
        Ok(this.configured(|converter| converter.document_language(language)))
    }

    fn add_font(&self, name: String, data: RString) -> Self {
        // SAFETY: `data` remains rooted for this call and is copied before returning.
        let bytes = unsafe { data.as_slice() }.to_vec();
        self.configured(|converter| converter.add_font(&name, bytes))
    }

    fn add_font_pack(ruby: &Ruby, this: &Self, kind: String, data: RString) -> Result<Self, Error> {
        let kind = kind
            .parse::<ironpress_core::FontPackKind>()
            .map_err(|error| Error::new(ruby.exception_arg_error(), error.to_string()))?;
        // SAFETY: `data` remains rooted for this call and is copied before returning.
        let bytes = unsafe { data.as_slice() }.to_vec();
        let pack = ironpress_core::FontPack::parse(kind, bytes)
            .map_err(|error| Error::new(ruby.exception_arg_error(), error.to_string()))?;
        Ok(this.configured(|converter| converter.add_font_pack(pack)))
    }

    fn base_path(&self, path: String) -> Self {
        self.configured(|converter| converter.base_path(Path::new(&path)))
    }

    fn resource_root(&self, path: String) -> Self {
        self.configured(|converter| converter.resource_root(Path::new(&path)))
    }

    fn header(&self, text: String) -> Self {
        self.configured(|converter| converter.header(text))
    }

    fn header_html(&self, html: String) -> Self {
        self.configured(|converter| converter.header_html(html))
    }

    fn footer(&self, text: String) -> Self {
        self.configured(|converter| converter.footer(text))
    }

    fn footer_html(&self, html: String) -> Self {
        self.configured(|converter| converter.footer_html(html))
    }

    fn convert(ruby: &Ruby, this: &Self, html: String) -> Result<RString, Error> {
        let pdf = this
            .converter
            .convert(&html)
            .map_err(|error| Error::new(ruby.exception_runtime_error(), error.to_string()))?;
        Ok(ruby.str_from_slice(&pdf))
    }

    fn convert_markdown(ruby: &Ruby, this: &Self, markdown: String) -> Result<RString, Error> {
        let pdf = this
            .converter
            .convert_markdown(&markdown)
            .map_err(|error| Error::new(ruby.exception_runtime_error(), error.to_string()))?;
        Ok(ruby.str_from_slice(&pdf))
    }

    fn convert_to_file(ruby: &Ruby, this: &Self, html: String, path: String) -> Result<(), Error> {
        let pdf = this
            .converter
            .convert(&html)
            .map_err(|error| Error::new(ruby.exception_runtime_error(), error.to_string()))?;
        std::fs::write(path, pdf)
            .map_err(|error| Error::new(ruby.exception_runtime_error(), error.to_string()))
    }

    fn convert_markdown_to_file(
        ruby: &Ruby,
        this: &Self,
        markdown: String,
        path: String,
    ) -> Result<(), Error> {
        let pdf = this
            .converter
            .convert_markdown(&markdown)
            .map_err(|error| Error::new(ruby.exception_runtime_error(), error.to_string()))?;
        std::fs::write(path, pdf)
            .map_err(|error| Error::new(ruby.exception_runtime_error(), error.to_string()))
    }
}

#[magnus::init]
fn init(_ruby: &Ruby) -> Result<(), Error> {
    let module = define_module("Ironpress")?;
    module.define_singleton_method("html_to_pdf", function!(html_to_pdf, 1))?;
    module.define_singleton_method("markdown_to_pdf", function!(markdown_to_pdf, 1))?;

    let class = module.define_class("NativeConverter", class::object())?;
    class.define_singleton_method("new", function!(NativeConverter::new, 0))?;
    class.define_method("page_size", method!(NativeConverter::page_size, 1))?;
    class.define_method(
        "page_size_custom",
        method!(NativeConverter::page_size_custom, 2),
    )?;
    class.define_method("margin", method!(NativeConverter::margin, 1))?;
    class.define_method("margin_sides", method!(NativeConverter::margin_sides, 4))?;
    class.define_method("compress", method!(NativeConverter::compress, 1))?;
    class.define_method("jpeg_quality", method!(NativeConverter::jpeg_quality, 1))?;
    class.define_method(
        "auto_resize_images",
        method!(NativeConverter::auto_resize_images, 1),
    )?;
    class.define_method("image_dpi", method!(NativeConverter::image_dpi, 1))?;
    class.define_method("filter_dpi", method!(NativeConverter::filter_dpi, 1))?;
    class.define_method("mask_dpi", method!(NativeConverter::mask_dpi, 1))?;
    class.define_method(
        "background_raster_dpi",
        method!(NativeConverter::background_raster_dpi, 1),
    )?;
    class.define_method(
        "occlusion_cull",
        method!(NativeConverter::occlusion_cull, 1),
    )?;
    class.define_method("sanitize", method!(NativeConverter::sanitize, 1))?;
    class.define_method("accessible", method!(NativeConverter::accessible, 1))?;
    class.define_method(
        "document_title",
        method!(NativeConverter::document_title, 1),
    )?;
    class.define_method(
        "document_language",
        method!(NativeConverter::document_language, 1),
    )?;
    class.define_method("add_font", method!(NativeConverter::add_font, 2))?;
    class.define_method("add_font_pack", method!(NativeConverter::add_font_pack, 2))?;
    class.define_method("base_path", method!(NativeConverter::base_path, 1))?;
    class.define_method("resource_root", method!(NativeConverter::resource_root, 1))?;
    class.define_method("header", method!(NativeConverter::header, 1))?;
    class.define_method("header_html", method!(NativeConverter::header_html, 1))?;
    class.define_method("footer", method!(NativeConverter::footer, 1))?;
    class.define_method("footer_html", method!(NativeConverter::footer_html, 1))?;
    class.define_method("convert", method!(NativeConverter::convert, 1))?;
    class.define_method(
        "convert_markdown",
        method!(NativeConverter::convert_markdown, 1),
    )?;
    class.define_method(
        "convert_to_file",
        method!(NativeConverter::convert_to_file, 2),
    )?;
    class.define_method(
        "convert_markdown_to_file",
        method!(NativeConverter::convert_markdown_to_file, 2),
    )?;
    Ok(())
}
