//! Hand-written bindings to the vendored meshoptimizer 1.3 (MIT,
//! `vendor/meshoptimizer`), only for the functions the normalizer uses.
//!
//! The crates.io `meshopt` 0.6 embeds 0.25, which has no `remesh` and no
//! `simplifyWithUpdate` (estudo 04 §5), so the C library is built here by
//! `build.rs` and wrapped with safe slices.

use std::os::raw::{c_float, c_int, c_uchar, c_uint, c_void};

pub const SIMPLIFY_LOCK_BORDER: u32 = 1 << 0;
pub const SIMPLIFY_ERROR_ABSOLUTE: u32 = 1 << 2;
pub const SIMPLIFY_PRUNE: u32 = 1 << 3;
pub const SIMPLIFY_PERMISSIVE: u32 = 1 << 5;
pub const SIMPLIFY_REGULARIZE_LIGHT: u32 = 1 << 6;
pub const SIMPLIFY_PRESERVE_FOLDS: u32 = 1 << 7;
pub const SIMPLIFY_ERROR_CLAMPED: u32 = 1 << 8;

pub const REMESH_SHELL: u32 = 1 << 0;
pub const REMESH_SOLVE: u32 = 1 << 1;

extern "C" {
    fn meshopt_generateVertexRemap(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertices: *const c_void,
        vertex_count: usize,
        vertex_size: usize,
    ) -> usize;
    fn meshopt_remapVertexBuffer(
        destination: *mut c_void,
        vertices: *const c_void,
        vertex_count: usize,
        vertex_size: usize,
        remap: *const c_uint,
    );
    fn meshopt_remapIndexBuffer(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        remap: *const c_uint,
    );
    fn meshopt_optimizeVertexCache(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertex_count: usize,
    );
    fn meshopt_optimizeVertexFetchRemap(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertex_count: usize,
    ) -> usize;
    fn meshopt_simplify(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertex_positions: *const c_float,
        vertex_count: usize,
        vertex_positions_stride: usize,
        target_index_count: usize,
        target_error: c_float,
        options: c_uint,
        result_error: *mut c_float,
    ) -> usize;
    fn meshopt_simplifyWithAttributes(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertex_positions: *const c_float,
        vertex_count: usize,
        vertex_positions_stride: usize,
        vertex_attributes: *const c_float,
        vertex_attributes_stride: usize,
        attribute_weights: *const c_float,
        attribute_count: usize,
        vertex_lock: *const c_uchar,
        target_index_count: usize,
        target_error: c_float,
        options: c_uint,
        result_error: *mut c_float,
    ) -> usize;
    fn meshopt_simplifyWithUpdate(
        indices: *mut c_uint,
        index_count: usize,
        vertex_positions: *mut c_float,
        vertex_count: usize,
        vertex_positions_stride: usize,
        vertex_attributes: *mut c_float,
        vertex_attributes_stride: usize,
        attribute_weights: *const c_float,
        attribute_count: usize,
        vertex_lock: *const c_uchar,
        target_index_count: usize,
        target_error: c_float,
        options: c_uint,
        result_error: *mut c_float,
    ) -> usize;
    fn meshopt_simplifySloppy(
        destination: *mut c_uint,
        indices: *const c_uint,
        index_count: usize,
        vertex_positions: *const c_float,
        vertex_count: usize,
        vertex_positions_stride: usize,
        vertex_lock: *const c_uchar,
        target_index_count: usize,
        target_error: c_float,
        result_error: *mut c_float,
    ) -> usize;
    fn meshopt_remesh(
        destination: *mut c_float,
        max_triangle_count: usize,
        indices: *const c_uint,
        index_count: usize,
        vertex_positions: *const c_float,
        vertex_count: usize,
        vertex_positions_stride: usize,
        resolution: c_int,
        options: c_uint,
    ) -> usize;
    fn meshopt_encodeVertexBufferBound(vertex_count: usize, vertex_size: usize) -> usize;
    fn meshopt_encodeVertexBufferLevel(
        buffer: *mut c_uchar,
        buffer_size: usize,
        vertices: *const c_void,
        vertex_count: usize,
        vertex_size: usize,
        level: c_int,
        version: c_int,
    ) -> usize;
    fn meshopt_encodeIndexBufferBound(index_count: usize, vertex_count: usize) -> usize;
    fn meshopt_encodeIndexBuffer(
        buffer: *mut c_uchar,
        buffer_size: usize,
        indices: *const c_uint,
        index_count: usize,
    ) -> usize;
    fn meshopt_encodeIndexSequenceBound(index_count: usize, vertex_count: usize) -> usize;
    fn meshopt_encodeIndexSequence(
        buffer: *mut c_uchar,
        buffer_size: usize,
        indices: *const c_uint,
        index_count: usize,
    ) -> usize;
    fn meshopt_decodeVertexBuffer(
        destination: *mut c_void,
        vertex_count: usize,
        vertex_size: usize,
        buffer: *const c_uchar,
        buffer_size: usize,
    ) -> c_int;
    fn meshopt_decodeIndexBuffer(
        destination: *mut c_void,
        index_count: usize,
        index_size: usize,
        buffer: *const c_uchar,
        buffer_size: usize,
    ) -> c_int;
    fn meshopt_decodeIndexSequence(
        destination: *mut c_void,
        index_count: usize,
        index_size: usize,
        buffer: *const c_uchar,
        buffer_size: usize,
    ) -> c_int;
    fn meshopt_encodeFilterQuat(
        destination: *mut c_void,
        count: usize,
        stride: usize,
        bits: c_int,
        data: *const c_float,
    );
    fn meshopt_encodeFilterExp(
        destination: *mut c_void,
        count: usize,
        stride: usize,
        bits: c_int,
        data: *const c_float,
        mode: c_int,
    );
    fn meshopt_decodeFilterOct(buffer: *mut c_void, count: usize, stride: usize);
    fn meshopt_decodeFilterQuat(buffer: *mut c_void, count: usize, stride: usize);
    fn meshopt_decodeFilterExp(buffer: *mut c_void, count: usize, stride: usize);
    fn meshopt_decodeFilterColor(buffer: *mut c_void, count: usize, stride: usize);
}

fn check_indices(indices: &[u32], vertex_count: usize) {
    assert!(
        indices.len().is_multiple_of(3),
        "index count must be a multiple of 3"
    );
    assert!(
        indices.iter().all(|&i| (i as usize) < vertex_count),
        "index out of range"
    );
}

/// Weld: remap table over a packed vertex stream (`vertex_size` bytes each).
/// Returns `(remap, unique_count)`.
pub fn generate_vertex_remap(
    indices: Option<&[u32]>,
    vertices: &[u8],
    vertex_size: usize,
) -> (Vec<u32>, usize) {
    assert!(vertex_size > 0 && vertices.len().is_multiple_of(vertex_size));
    let vertex_count = vertices.len() / vertex_size;
    let index_count = indices.map(|i| i.len()).unwrap_or(vertex_count);
    if let Some(ix) = indices {
        check_indices(ix, vertex_count);
    }
    let mut remap = vec![0u32; vertex_count];
    let unique = unsafe {
        meshopt_generateVertexRemap(
            remap.as_mut_ptr(),
            indices.map(|i| i.as_ptr()).unwrap_or(std::ptr::null()),
            index_count,
            vertices.as_ptr() as *const c_void,
            vertex_count,
            vertex_size,
        )
    };
    (remap, unique)
}

pub fn remap_vertex_buffer<T: Copy + Default>(
    vertices: &[T],
    unique: usize,
    remap: &[u32],
) -> Vec<T> {
    assert_eq!(vertices.len(), remap.len());
    let mut out = vec![T::default(); unique];
    unsafe {
        meshopt_remapVertexBuffer(
            out.as_mut_ptr() as *mut c_void,
            vertices.as_ptr() as *const c_void,
            vertices.len(),
            std::mem::size_of::<T>(),
            remap.as_ptr(),
        );
    }
    out
}

pub fn remap_index_buffer(indices: Option<&[u32]>, vertex_count: usize, remap: &[u32]) -> Vec<u32> {
    let n = indices.map(|i| i.len()).unwrap_or(vertex_count);
    let mut out = vec![0u32; n];
    unsafe {
        meshopt_remapIndexBuffer(
            out.as_mut_ptr(),
            indices.map(|i| i.as_ptr()).unwrap_or(std::ptr::null()),
            n,
            remap.as_ptr(),
        );
    }
    out
}

pub fn optimize_vertex_cache(indices: &mut [u32], vertex_count: usize) {
    check_indices(indices, vertex_count);
    let src = indices.to_vec();
    unsafe {
        meshopt_optimizeVertexCache(indices.as_mut_ptr(), src.as_ptr(), src.len(), vertex_count);
    }
}

/// Remap table that orders vertices by first use (fetch locality). Returns
/// `(remap, unique_count)`; unused vertices map to `u32::MAX`.
pub fn optimize_vertex_fetch_remap(indices: &[u32], vertex_count: usize) -> (Vec<u32>, usize) {
    check_indices(indices, vertex_count);
    let mut remap = vec![0u32; vertex_count];
    let unique = unsafe {
        meshopt_optimizeVertexFetchRemap(
            remap.as_mut_ptr(),
            indices.as_ptr(),
            indices.len(),
            vertex_count,
        )
    };
    (remap, unique)
}

/// `meshopt_simplify` over tightly packed `[f32; 3]` positions.
pub fn simplify(
    indices: &[u32],
    positions: &[[f32; 3]],
    target_index_count: usize,
    target_error: f32,
    options: u32,
) -> (Vec<u32>, f32) {
    check_indices(indices, positions.len());
    let mut out = vec![0u32; indices.len()];
    let mut err = 0f32;
    let n = unsafe {
        meshopt_simplify(
            out.as_mut_ptr(),
            indices.as_ptr(),
            indices.len(),
            positions.as_ptr() as *const c_float,
            positions.len(),
            12,
            target_index_count,
            target_error,
            options,
            &mut err,
        )
    };
    out.truncate(n);
    (out, err)
}

/// `meshopt_simplifyWithAttributes`. `attributes` holds `weights.len()` floats
/// per vertex; `locks` is empty or one flag byte per vertex.
#[allow(clippy::too_many_arguments)]
pub fn simplify_with_attributes(
    indices: &[u32],
    positions: &[[f32; 3]],
    attributes: &[f32],
    weights: &[f32],
    locks: &[u8],
    target_index_count: usize,
    target_error: f32,
    options: u32,
) -> (Vec<u32>, f32) {
    check_indices(indices, positions.len());
    let ac = weights.len();
    assert!(ac <= 32 && attributes.len() == positions.len() * ac);
    assert!(locks.is_empty() || locks.len() == positions.len());
    let mut out = vec![0u32; indices.len()];
    let mut err = 0f32;
    let n = unsafe {
        meshopt_simplifyWithAttributes(
            out.as_mut_ptr(),
            indices.as_ptr(),
            indices.len(),
            positions.as_ptr() as *const c_float,
            positions.len(),
            12,
            if ac == 0 {
                std::ptr::null()
            } else {
                attributes.as_ptr()
            },
            ac * 4,
            if ac == 0 {
                std::ptr::null()
            } else {
                weights.as_ptr()
            },
            ac,
            if locks.is_empty() {
                std::ptr::null()
            } else {
                locks.as_ptr()
            },
            target_index_count,
            target_error,
            options,
            &mut err,
        )
    };
    out.truncate(n);
    (out, err)
}

/// `meshopt_simplifyWithUpdate`: moves positions and attributes in place.
/// Never used on skinned meshes (it would break the bind).
#[allow(clippy::too_many_arguments)]
pub fn simplify_with_update(
    indices: &mut Vec<u32>,
    positions: &mut [[f32; 3]],
    attributes: &mut [f32],
    weights: &[f32],
    target_index_count: usize,
    target_error: f32,
    options: u32,
) -> f32 {
    check_indices(indices, positions.len());
    let ac = weights.len();
    assert!(ac <= 32 && attributes.len() == positions.len() * ac);
    let mut err = 0f32;
    let n = unsafe {
        meshopt_simplifyWithUpdate(
            indices.as_mut_ptr(),
            indices.len(),
            positions.as_mut_ptr() as *mut c_float,
            positions.len(),
            12,
            if ac == 0 {
                std::ptr::null_mut()
            } else {
                attributes.as_mut_ptr()
            },
            ac * 4,
            if ac == 0 {
                std::ptr::null()
            } else {
                weights.as_ptr()
            },
            ac,
            std::ptr::null(),
            target_index_count,
            target_error,
            options,
            &mut err,
        )
    };
    indices.truncate(n);
    err
}

pub fn simplify_sloppy(
    indices: &[u32],
    positions: &[[f32; 3]],
    target_index_count: usize,
    target_error: f32,
) -> (Vec<u32>, f32) {
    check_indices(indices, positions.len());
    let mut out = vec![0u32; indices.len()];
    let mut err = 0f32;
    let n = unsafe {
        meshopt_simplifySloppy(
            out.as_mut_ptr(),
            indices.as_ptr(),
            indices.len(),
            positions.as_ptr() as *const c_float,
            positions.len(),
            12,
            std::ptr::null(),
            target_index_count,
            target_error,
            &mut err,
        )
    };
    out.truncate(n);
    (out, err)
}

/// Voxel remesh (experimental in 1.3). Returns an unindexed triangle soup,
/// three positions per triangle. `resolution` is clamped to 4..=256.
pub fn remesh(
    indices: &[u32],
    positions: &[[f32; 3]],
    resolution: i32,
    options: u32,
) -> Vec<[f32; 3]> {
    check_indices(indices, positions.len());
    let res = resolution.clamp(4, 256);
    let bound = unsafe {
        meshopt_remesh(
            std::ptr::null_mut(),
            0,
            indices.as_ptr(),
            indices.len(),
            positions.as_ptr() as *const c_float,
            positions.len(),
            12,
            res,
            options,
        )
    };
    let mut out = vec![[0f32; 3]; bound * 3];
    let n = unsafe {
        meshopt_remesh(
            out.as_mut_ptr() as *mut c_float,
            bound,
            indices.as_ptr(),
            indices.len(),
            positions.as_ptr() as *const c_float,
            positions.len(),
            12,
            res,
            options,
        )
    };
    out.truncate(n.min(bound) * 3);
    out
}

/// Vertex codec, format version 0 (the one `EXT_meshopt_compression` allows).
pub fn encode_vertex_buffer(vertices: &[u8], vertex_size: usize) -> Vec<u8> {
    assert!(vertex_size.is_multiple_of(4) && vertex_size <= 256 && vertex_size > 0);
    assert!(vertices.len().is_multiple_of(vertex_size));
    let count = vertices.len() / vertex_size;
    let bound = unsafe { meshopt_encodeVertexBufferBound(count, vertex_size) };
    let mut out = vec![0u8; bound];
    let n = unsafe {
        meshopt_encodeVertexBufferLevel(
            out.as_mut_ptr(),
            bound,
            vertices.as_ptr() as *const c_void,
            count,
            vertex_size,
            2,
            0,
        )
    };
    assert!(n > 0, "meshopt vertex encode failed");
    out.truncate(n);
    out
}

/// Triangle index codec (mode TRIANGLES).
pub fn encode_index_buffer(indices: &[u32], vertex_count: usize) -> Vec<u8> {
    check_indices(indices, vertex_count);
    let bound = unsafe { meshopt_encodeIndexBufferBound(indices.len(), vertex_count) };
    let mut out = vec![0u8; bound];
    let n = unsafe {
        meshopt_encodeIndexBuffer(out.as_mut_ptr(), bound, indices.as_ptr(), indices.len())
    };
    assert!(n > 0, "meshopt index encode failed");
    out.truncate(n);
    out
}

pub fn encode_index_sequence(indices: &[u32], vertex_count: usize) -> Vec<u8> {
    let bound = unsafe { meshopt_encodeIndexSequenceBound(indices.len(), vertex_count) };
    let mut out = vec![0u8; bound];
    let n = unsafe {
        meshopt_encodeIndexSequence(out.as_mut_ptr(), bound, indices.as_ptr(), indices.len())
    };
    assert!(n > 0, "meshopt index sequence encode failed");
    out.truncate(n);
    out
}

/// Decoders are safe for untrusted input (they may produce garbage, never
/// read or write out of bounds), which is what an imported GLB needs.
pub fn decode_vertex_buffer(count: usize, size: usize, data: &[u8]) -> Option<Vec<u8>> {
    let mut out = vec![0u8; count.checked_mul(size)?];
    let rc = unsafe {
        meshopt_decodeVertexBuffer(
            out.as_mut_ptr() as *mut c_void,
            count,
            size,
            data.as_ptr(),
            data.len(),
        )
    };
    (rc == 0).then_some(out)
}

pub fn decode_index_buffer(count: usize, size: usize, data: &[u8]) -> Option<Vec<u8>> {
    if size != 2 && size != 4 {
        return None;
    }
    let mut out = vec![0u8; count.checked_mul(size)?];
    let rc = unsafe {
        meshopt_decodeIndexBuffer(
            out.as_mut_ptr() as *mut c_void,
            count,
            size,
            data.as_ptr(),
            data.len(),
        )
    };
    (rc == 0).then_some(out)
}

pub fn decode_index_sequence(count: usize, size: usize, data: &[u8]) -> Option<Vec<u8>> {
    if size != 2 && size != 4 {
        return None;
    }
    let mut out = vec![0u8; count.checked_mul(size)?];
    let rc = unsafe {
        meshopt_decodeIndexSequence(
            out.as_mut_ptr() as *mut c_void,
            count,
            size,
            data.as_ptr(),
            data.len(),
        )
    };
    (rc == 0).then_some(out)
}

/// In-place filter decode; `filter` is the glTF name (OCTAHEDRAL, QUATERNION,
/// EXPONENTIAL, COLOR). Returns false for an unknown filter or bad stride.
pub fn decode_filter(filter: &str, data: &mut [u8], count: usize, stride: usize) -> bool {
    if stride == 0 || data.len() < count.saturating_mul(stride) {
        return false;
    }
    let p = data.as_mut_ptr() as *mut c_void;
    match filter {
        "OCTAHEDRAL" if stride == 4 || stride == 8 => unsafe {
            meshopt_decodeFilterOct(p, count, stride)
        },
        "QUATERNION" if stride == 8 => unsafe { meshopt_decodeFilterQuat(p, count, stride) },
        "EXPONENTIAL" if stride.is_multiple_of(4) => unsafe {
            meshopt_decodeFilterExp(p, count, stride)
        },
        "COLOR" if stride == 4 || stride == 8 => unsafe {
            meshopt_decodeFilterColor(p, count, stride)
        },
        "NONE" => {}
        _ => return false,
    }
    true
}

/// Unit quaternions (xyzw) to the 16-bit QUATERNION filter format (stride 8).
pub fn encode_filter_quat(quats: &[[f32; 4]], bits: i32) -> Vec<u8> {
    let mut out = vec![0u8; quats.len() * 8];
    unsafe {
        meshopt_encodeFilterQuat(
            out.as_mut_ptr() as *mut c_void,
            quats.len(),
            8,
            bits.clamp(4, 16),
            quats.as_ptr() as *const c_float,
        );
    }
    out
}

/// Floats (`comps` per element) to the EXPONENTIAL filter format with a
/// shared exponent per vector; stride `comps * 4`.
pub fn encode_filter_exp(data: &[f32], comps: usize, bits: i32) -> Vec<u8> {
    assert!(comps > 0 && data.len().is_multiple_of(comps));
    let count = data.len() / comps;
    let mut out = vec![0u8; data.len() * 4];
    unsafe {
        meshopt_encodeFilterExp(
            out.as_mut_ptr() as *mut c_void,
            count,
            comps * 4,
            bits.clamp(1, 24),
            data.as_ptr() as *const c_float,
            1, // meshopt_EncodeExpSharedVector
        );
    }
    out
}
