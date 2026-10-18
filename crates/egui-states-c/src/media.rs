//! Images and numeric data buffers.
//!
//! While a client is connected, these functions may block until the state's
//! previous transfer is acknowledged or the client disconnects.

use std::ffi::c_void;

use egui_states::c_abi::{CServer, DataKind, Error, ErrorCode, ImageFormat, ImageInput};

use crate::ffi::{
    bytes, data_kind, egui_states_status_t, elements, ffi, output, output_elements, reference,
    too_small, write, write_optional,
};
use crate::meta::{
    EGUI_STATES_BLOCKING, EGUI_STATES_CACHE, EGUI_STATES_FORCE, EGUI_STATES_UPDATE,
    egui_states_data_type_t, egui_states_image_format_t,
};
use crate::server::{egui_states_server_t, server_ref};
use crate::states::has;

/// Image pixels passed to the server.
///
/// Rows start `row_stride` bytes apart; 0 means tightly packed rows of
/// `width` pixels. `data` must hold at least
/// `(height - 1) * stride + width * bytes per pixel` bytes, and `data_len`
/// reports how many it holds. Both dimensions must be between 1 and 65,536.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct egui_states_image_t {
    /// First byte of the first row.
    pub data: *const u8,
    /// Number of readable bytes at `data`.
    pub data_len: usize,
    /// Number of rows.
    pub height: usize,
    /// Number of pixels per row.
    pub width: usize,
    /// Bytes between the starts of consecutive rows, or 0 when packed.
    pub row_stride: usize,
    /// Channel layout, one of `EGUI_STATES_IMAGE_*`.
    pub format: egui_states_image_format_t,
}

fn image_input<'a>(image: *const egui_states_image_t) -> Result<ImageInput<'a>, Error> {
    let image = unsafe { reference(image, "image") }?;
    let format = ImageFormat::from_id(image.format).ok_or_else(|| {
        Error::new(
            ErrorCode::InvalidArgument,
            format!("{} is not an EGUI_STATES_IMAGE_* value", image.format),
        )
    })?;
    Ok(ImageInput {
        data: unsafe { bytes(image.data, image.data_len, "image.data") }?,
        height: image.height,
        width: image.width,
        row_stride: image.row_stride,
        format,
    })
}

/// Copies pixels with `copy` and reports the size; a too small buffer is an
/// error after the outputs are written.
fn copy_image(
    out: *mut u8,
    capacity: usize,
    out_required: *mut usize,
    out_height: *mut usize,
    out_width: *mut usize,
    copy: impl FnOnce(Option<&mut [u8]>) -> Result<(usize, [usize; 2], bool), Error>,
) -> Result<(), Error> {
    output(out_required, "out_required")?;
    let buffer = unsafe { output_elements(out.cast(), capacity, 1, "out") }?;
    let requested = buffer.is_some();
    let (required, [height, width], copied) = copy(buffer)?;
    unsafe {
        out_required.write(required);
        write_optional(out_height, height, "out_height")?;
        write_optional(out_width, width, "out_width")?;
    }
    if requested && !copied {
        return Err(too_small(capacity, required, "bytes"));
    }
    Ok(())
}

/// Copies numbers with `copy` and reports their count; a too small buffer is
/// an error after the count is written.
fn copy_data(
    data_type: egui_states_data_type_t,
    out: *mut c_void,
    capacity: usize,
    out_count: *mut usize,
    copy: impl FnOnce(DataKind, Option<&mut [u8]>) -> Result<(usize, bool), Error>,
) -> Result<(), Error> {
    output(out_count, "out_count")?;
    let kind = data_kind(data_type)?;
    let buffer = unsafe { output_elements(out, capacity, kind.size(), "out") }?;
    let requested = buffer.is_some();
    let (count, copied) = copy(kind, buffer)?;
    unsafe { out_count.write(count) };
    if requested && !copied {
        return Err(too_small(capacity, count, "elements"));
    }
    Ok(())
}

fn input_data<'a>(
    data_type: egui_states_data_type_t,
    data: *const c_void,
    count: usize,
) -> Result<(DataKind, &'a [u8]), Error> {
    let kind = data_kind(data_type)?;
    Ok((kind, unsafe { elements(data, count, kind.size(), "data") }?))
}

fn update_flag(flags: u32) -> Result<bool, Error> {
    has(flags, EGUI_STATES_UPDATE, EGUI_STATES_UPDATE)
}

/// Splits take flags into `(blocking, update, cache)`.
fn take_flags(flags: u32, allowed: u32) -> Result<(bool, bool, bool), Error> {
    let blocking = has(flags, allowed, EGUI_STATES_BLOCKING)?;
    Ok((
        blocking,
        flags & EGUI_STATES_UPDATE != 0,
        flags & EGUI_STATES_CACHE != 0,
    ))
}

fn run(
    server: *const egui_states_server_t,
    body: impl FnOnce(&CServer) -> Result<(), Error>,
) -> egui_states_status_t {
    ffi(|| body(server_ref(server)?))
}

// image -----------------------------------------------------------------------

/// Reports the shape of an `Image`; both are 0 before the first set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_image_size(
    server: *const egui_states_server_t,
    id: u64,
    out_height: *mut usize,
    out_width: *mut usize,
) -> egui_states_status_t {
    run(server, |server| {
        output(out_height, "out_height")?;
        output(out_width, "out_width")?;
        let [height, width] = server.image_size(id)?;
        unsafe {
            write(out_height, height, "out_height")?;
            write(out_width, width, "out_width")
        }
    })
}

/// Copies the RGBA pixels of an `Image` into `out` of `capacity` bytes.
///
/// `out_required` receives the byte count, and the non-null `out_height` and
/// `out_width` the shape, even when the buffer is too small. A null `out`
/// with zero capacity asks only for the size.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_image_get(
    server: *const egui_states_server_t,
    id: u64,
    out: *mut u8,
    capacity: usize,
    out_required: *mut usize,
    out_height: *mut usize,
    out_width: *mut usize,
) -> egui_states_status_t {
    run(server, |server| {
        copy_image(
            out,
            capacity,
            out_required,
            out_height,
            out_width,
            |buffer| server.image_get(id, buffer),
        )
    })
}

/// Replaces an `Image`. `flags` may be `EGUI_STATES_UPDATE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_image_set(
    server: *const egui_states_server_t,
    id: u64,
    image: *const egui_states_image_t,
    flags: u32,
) -> egui_states_status_t {
    run(server, |server| {
        server.image_set(id, &image_input(image)?, update_flag(flags)?)
    })
}

/// Resizes an `Image` to `height` x `width` and fills it with one RGBA color.
/// `flags` may be `EGUI_STATES_UPDATE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_image_set_all(
    server: *const egui_states_server_t,
    id: u64,
    height: usize,
    width: usize,
    red: u8,
    green: u8,
    blue: u8,
    alpha: u8,
    flags: u32,
) -> egui_states_status_t {
    run(server, |server| {
        let rgba = [red, green, blue, alpha];
        server.image_set_all(id, [height, width], rgba, update_flag(flags)?)
    })
}

/// Replaces the rectangle of an `Image` starting at row `y` and column `x`;
/// it must lie inside the image. `flags` may combine `EGUI_STATES_UPDATE` and
/// `EGUI_STATES_FORCE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_image_update(
    server: *const egui_states_server_t,
    id: u64,
    image: *const egui_states_image_t,
    y: usize,
    x: usize,
    flags: u32,
) -> egui_states_status_t {
    run(server, |server| {
        let force = has(
            flags,
            EGUI_STATES_UPDATE | EGUI_STATES_FORCE,
            EGUI_STATES_FORCE,
        )?;
        let update = flags & EGUI_STATES_UPDATE != 0;
        server.image_update(id, &image_input(image)?, [y, x], update, force)
    })
}

// image multi -----------------------------------------------------------------

/// Reports the shape of the image at `index` of an `ImageMulti`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_image_multi_size(
    server: *const egui_states_server_t,
    id: u64,
    index: u32,
    out_height: *mut usize,
    out_width: *mut usize,
) -> egui_states_status_t {
    run(server, |server| {
        output(out_height, "out_height")?;
        output(out_width, "out_width")?;
        let [height, width] = server.image_multi_size(id, index)?;
        unsafe {
            write(out_height, height, "out_height")?;
            write(out_width, width, "out_width")
        }
    })
}

/// Copies the RGBA pixels at `index` of an `ImageMulti`, like
/// `egui_states_image_get`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_image_multi_get(
    server: *const egui_states_server_t,
    id: u64,
    index: u32,
    out: *mut u8,
    capacity: usize,
    out_required: *mut usize,
    out_height: *mut usize,
    out_width: *mut usize,
) -> egui_states_status_t {
    run(server, |server| {
        copy_image(
            out,
            capacity,
            out_required,
            out_height,
            out_width,
            |buffer| server.image_multi_get(id, index, buffer),
        )
    })
}

/// Replaces or creates the image at `index`. `flags` may be `EGUI_STATES_UPDATE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_image_multi_set(
    server: *const egui_states_server_t,
    id: u64,
    index: u32,
    image: *const egui_states_image_t,
    flags: u32,
) -> egui_states_status_t {
    run(server, |server| {
        server.image_multi_set(id, index, &image_input(image)?, update_flag(flags)?)
    })
}

/// Replaces or creates the image at `index`, filled with one RGBA color.
/// `flags` may be `EGUI_STATES_UPDATE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_image_multi_set_all(
    server: *const egui_states_server_t,
    id: u64,
    index: u32,
    height: usize,
    width: usize,
    red: u8,
    green: u8,
    blue: u8,
    alpha: u8,
    flags: u32,
) -> egui_states_status_t {
    run(server, |server| {
        let rgba = [red, green, blue, alpha];
        let update = update_flag(flags)?;
        server.image_multi_set_all(id, index, [height, width], rgba, update)
    })
}

/// Replaces a rectangle of the existing image at `index`, like
/// `egui_states_image_update`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_image_multi_update(
    server: *const egui_states_server_t,
    id: u64,
    index: u32,
    image: *const egui_states_image_t,
    y: usize,
    x: usize,
    flags: u32,
) -> egui_states_status_t {
    run(server, |server| {
        let force = has(
            flags,
            EGUI_STATES_UPDATE | EGUI_STATES_FORCE,
            EGUI_STATES_FORCE,
        )?;
        let update = flags & EGUI_STATES_UPDATE != 0;
        server.image_multi_update(id, index, &image_input(image)?, [y, x], update, force)
    })
}

/// Removes the image at `index`; a missing index is ignored. `flags` may be
/// `EGUI_STATES_UPDATE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_image_multi_remove_index(
    server: *const egui_states_server_t,
    id: u64,
    index: u32,
    flags: u32,
) -> egui_states_status_t {
    run(server, |server| {
        server.image_multi_remove_index(id, index, update_flag(flags)?)
    })
}

/// Removes every image. `flags` may be `EGUI_STATES_UPDATE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_image_multi_reset(
    server: *const egui_states_server_t,
    id: u64,
    flags: u32,
) -> egui_states_status_t {
    run(server, |server| {
        server.image_multi_reset(id, update_flag(flags)?)
    })
}

/// Reports the number of images.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_image_multi_len(
    server: *const egui_states_server_t,
    id: u64,
    out_len: *mut usize,
) -> egui_states_status_t {
    run(server, |server| {
        output(out_len, "out_len")?;
        let len = server.image_multi_len(id)?;
        unsafe { write(out_len, len, "out_len") }
    })
}

/// Reports whether an image exists at `index`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_image_multi_contains(
    server: *const egui_states_server_t,
    id: u64,
    index: u32,
    out_contains: *mut bool,
) -> egui_states_status_t {
    run(server, |server| {
        output(out_contains, "out_contains")?;
        let contains = server.image_multi_contains(id, index)?;
        unsafe { write(out_contains, contains, "out_contains") }
    })
}

/// Copies the indices of every image, in unspecified order, into `out` of
/// `capacity` elements. `out_required` receives their count even when the
/// buffer is too small; a null `out` with zero capacity asks only for it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_image_multi_indices(
    server: *const egui_states_server_t,
    id: u64,
    out: *mut u32,
    capacity: usize,
    out_required: *mut usize,
) -> egui_states_status_t {
    run(server, |server| {
        output(out_required, "out_required")?;
        let buffer = unsafe { output_elements(out.cast(), capacity, 4, "out") }?;
        let indices = server.image_multi_indices(id)?;
        unsafe { out_required.write(indices.len()) };
        if let Some(buffer) = buffer {
            if capacity < indices.len() {
                return Err(too_small(capacity, indices.len(), "elements"));
            }
            for (chunk, index) in buffer.as_chunks_mut::<4>().0.iter_mut().zip(&indices) {
                chunk.copy_from_slice(&index.to_ne_bytes());
            }
        }
        Ok(())
    })
}

// data ------------------------------------------------------------------------

/// Reports the number of elements of a `Data` state.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_data_len(
    server: *const egui_states_server_t,
    id: u64,
    out_count: *mut usize,
) -> egui_states_status_t {
    run(server, |server| {
        output(out_count, "out_count")?;
        let count = server.data_len(id)?;
        unsafe { write(out_count, count, "out_count") }
    })
}

/// Copies the elements of a `Data` state into `out` of `capacity` elements of
/// `data_type`, which must be the registered type.
///
/// `out_count` receives the element count even when the buffer is too small;
/// a null `out` with zero capacity asks only for it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_data_get(
    server: *const egui_states_server_t,
    id: u64,
    data_type: egui_states_data_type_t,
    out: *mut c_void,
    capacity: usize,
    out_count: *mut usize,
) -> egui_states_status_t {
    run(server, |server| {
        copy_data(data_type, out, capacity, out_count, |kind, buffer| {
            server.data_get(id, kind, buffer)
        })
    })
}

/// Replaces the elements of a `Data` state with `count` elements of
/// `data_type` at `data`. `flags` may be `EGUI_STATES_UPDATE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_data_set(
    server: *const egui_states_server_t,
    id: u64,
    data_type: egui_states_data_type_t,
    data: *const c_void,
    count: usize,
    flags: u32,
) -> egui_states_status_t {
    run(server, |server| {
        let (kind, data) = input_data(data_type, data, count)?;
        server.data_set(id, kind, data, update_flag(flags)?)
    })
}

/// Appends `count` elements. `flags` may be `EGUI_STATES_UPDATE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_data_add(
    server: *const egui_states_server_t,
    id: u64,
    data_type: egui_states_data_type_t,
    data: *const c_void,
    count: usize,
    flags: u32,
) -> egui_states_status_t {
    run(server, |server| {
        let (kind, data) = input_data(data_type, data, count)?;
        server.data_add(id, kind, data, update_flag(flags)?)
    })
}

/// Overwrites `count` elements starting at element `index`; they must lie
/// inside the buffer. `flags` may be `EGUI_STATES_UPDATE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_data_replace(
    server: *const egui_states_server_t,
    id: u64,
    data_type: egui_states_data_type_t,
    data: *const c_void,
    count: usize,
    index: usize,
    flags: u32,
) -> egui_states_status_t {
    run(server, |server| {
        let (kind, data) = input_data(data_type, data, count)?;
        server.data_replace(id, kind, data, index, update_flag(flags)?)
    })
}

/// Removes `count` (at least 1) elements starting at element `index`.
/// `flags` may be `EGUI_STATES_UPDATE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_data_remove(
    server: *const egui_states_server_t,
    id: u64,
    index: usize,
    count: usize,
    flags: u32,
) -> egui_states_status_t {
    run(server, |server| {
        server.data_remove(id, index, count, update_flag(flags)?)
    })
}

/// Removes every element. `flags` may be `EGUI_STATES_UPDATE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_data_clear(
    server: *const egui_states_server_t,
    id: u64,
    flags: u32,
) -> egui_states_status_t {
    run(server, |server| server.data_clear(id, update_flag(flags)?))
}

/// Sends a one-shot buffer to a `DataTake` state.
///
/// `flags` may combine `EGUI_STATES_BLOCKING`, `EGUI_STATES_UPDATE`, and
/// `EGUI_STATES_CACHE`, which keeps the buffer for clients that connect later.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_data_take_set(
    server: *const egui_states_server_t,
    id: u64,
    data_type: egui_states_data_type_t,
    data: *const c_void,
    count: usize,
    flags: u32,
) -> egui_states_status_t {
    run(server, |server| {
        let allowed = EGUI_STATES_BLOCKING | EGUI_STATES_UPDATE | EGUI_STATES_CACHE;
        let (blocking, update, cache) = take_flags(flags, allowed)?;
        let (kind, data) = input_data(data_type, data, count)?;
        server.data_take_set(id, kind, data, blocking, update, cache)
    })
}

// data multi ------------------------------------------------------------------

/// Copies the elements at `index` of a `DataMulti` state, like
/// `egui_states_data_get`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_data_multi_get(
    server: *const egui_states_server_t,
    id: u64,
    index: u32,
    data_type: egui_states_data_type_t,
    out: *mut c_void,
    capacity: usize,
    out_count: *mut usize,
) -> egui_states_status_t {
    run(server, |server| {
        copy_data(data_type, out, capacity, out_count, |kind, buffer| {
            server.data_multi_get(id, index, kind, buffer)
        })
    })
}

/// Replaces or creates the buffer at `index`. `flags` may be `EGUI_STATES_UPDATE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_data_multi_set(
    server: *const egui_states_server_t,
    id: u64,
    index: u32,
    data_type: egui_states_data_type_t,
    data: *const c_void,
    count: usize,
    flags: u32,
) -> egui_states_status_t {
    run(server, |server| {
        let (kind, data) = input_data(data_type, data, count)?;
        server.data_multi_set(id, index, kind, data, update_flag(flags)?)
    })
}

/// Appends elements to the existing buffer at `index`. `flags` may be
/// `EGUI_STATES_UPDATE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_data_multi_add(
    server: *const egui_states_server_t,
    id: u64,
    index: u32,
    data_type: egui_states_data_type_t,
    data: *const c_void,
    count: usize,
    flags: u32,
) -> egui_states_status_t {
    run(server, |server| {
        let (kind, data) = input_data(data_type, data, count)?;
        server.data_multi_add(id, index, kind, data, update_flag(flags)?)
    })
}

/// Overwrites elements of the existing buffer at `index` starting at element
/// `data_index`. `flags` may be `EGUI_STATES_UPDATE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_data_multi_replace(
    server: *const egui_states_server_t,
    id: u64,
    index: u32,
    data_type: egui_states_data_type_t,
    data: *const c_void,
    count: usize,
    data_index: usize,
    flags: u32,
) -> egui_states_status_t {
    run(server, |server| {
        let (kind, data) = input_data(data_type, data, count)?;
        let update = update_flag(flags)?;
        server.data_multi_replace(id, index, kind, data, data_index, update)
    })
}

/// Removes `count` (at least 1) elements of the existing buffer at `index`
/// starting at element `data_index`. `flags` may be `EGUI_STATES_UPDATE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_data_multi_remove(
    server: *const egui_states_server_t,
    id: u64,
    index: u32,
    data_index: usize,
    count: usize,
    flags: u32,
) -> egui_states_status_t {
    run(server, |server| {
        let update = update_flag(flags)?;
        server.data_multi_remove(id, index, data_index, count, update)
    })
}

/// Removes every element of the existing buffer at `index`, keeping the
/// index. `flags` may be `EGUI_STATES_UPDATE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_data_multi_clear(
    server: *const egui_states_server_t,
    id: u64,
    index: u32,
    flags: u32,
) -> egui_states_status_t {
    run(server, |server| {
        server.data_multi_clear(id, index, update_flag(flags)?)
    })
}

/// Removes the buffer at `index`; a missing index is ignored. `flags` may be
/// `EGUI_STATES_UPDATE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_data_multi_remove_index(
    server: *const egui_states_server_t,
    id: u64,
    index: u32,
    flags: u32,
) -> egui_states_status_t {
    run(server, |server| {
        server.data_multi_remove_index(id, index, update_flag(flags)?)
    })
}

/// Removes every buffer. `flags` may be `EGUI_STATES_UPDATE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_data_multi_reset(
    server: *const egui_states_server_t,
    id: u64,
    flags: u32,
) -> egui_states_status_t {
    run(server, |server| {
        server.data_multi_reset(id, update_flag(flags)?)
    })
}

/// Sends a one-shot buffer at `index` of a `DataMultiTake` state, with the
/// flags of `egui_states_data_take_set`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_data_multi_take_set(
    server: *const egui_states_server_t,
    id: u64,
    index: u32,
    data_type: egui_states_data_type_t,
    data: *const c_void,
    count: usize,
    flags: u32,
) -> egui_states_status_t {
    run(server, |server| {
        let allowed = EGUI_STATES_BLOCKING | EGUI_STATES_UPDATE | EGUI_STATES_CACHE;
        let (blocking, update, cache) = take_flags(flags, allowed)?;
        let (kind, data) = input_data(data_type, data, count)?;
        server.data_multi_take_set(id, index, kind, data, blocking, update, cache)
    })
}

/// Removes the one-shot buffer at `index`; a missing index is ignored.
/// `flags` may be `EGUI_STATES_UPDATE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_data_multi_take_remove_index(
    server: *const egui_states_server_t,
    id: u64,
    index: u32,
    flags: u32,
) -> egui_states_status_t {
    run(server, |server| {
        server.data_multi_take_remove_index(id, index, update_flag(flags)?)
    })
}

/// Removes every one-shot buffer. `flags` may be `EGUI_STATES_UPDATE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn egui_states_data_multi_take_reset(
    server: *const egui_states_server_t,
    id: u64,
    flags: u32,
) -> egui_states_status_t {
    run(server, |server| {
        server.data_multi_take_reset(id, update_flag(flags)?)
    })
}
