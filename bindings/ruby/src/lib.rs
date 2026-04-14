use magnus::{Error, RString, Ruby, define_module, function, prelude::*};

fn html_to_pdf(ruby: &Ruby, html: String) -> Result<RString, Error> {
    let pdf = ironpress_core::html_to_pdf(&html)
        .map_err(|e| Error::new(ruby.exception_runtime_error(), e.to_string()))?;
    Ok(ruby.str_from_slice(&pdf))
}

fn markdown_to_pdf(ruby: &Ruby, markdown: String) -> Result<RString, Error> {
    let pdf = ironpress_core::markdown_to_pdf(&markdown)
        .map_err(|e| Error::new(ruby.exception_runtime_error(), e.to_string()))?;
    Ok(ruby.str_from_slice(&pdf))
}

#[magnus::init]
fn init(ruby: &Ruby) -> Result<(), Error> {
    let module = define_module("Ironpress")?;
    module.define_singleton_method("html_to_pdf", function!(html_to_pdf, 1))?;
    module.define_singleton_method("markdown_to_pdf", function!(markdown_to_pdf, 1))?;
    Ok(())
}
