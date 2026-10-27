// Upscale 4K: page upscaling in the spirit of Anime4K, written for comic
// and manga pages. Line art is upscaled along its edges, then its lines
// are retraced (the soft pixels an enlargement leaves on either side of an
// edge are pushed to one side or the other) and kept thin, then sharpened
// a little. Colour artwork gets the edge-aware upscale and the sharpening
// only, like AMD's FSR 1.
//
// Pixels are packed RGBA8 in a u32, with the pixel's brightness (luma) in
// the alpha byte, so each step gets it for free. Each pass reads one
// buffer and writes another: unpack the source; upscale it; for line art,
// gradient, retrace and thin; then sharpen.

struct Params {
    iw: u32,
    ih: u32,
    ow: u32,
    oh: u32,
    // The source rectangle the output covers: x0, y0, x1, y1.
    rect: vec4<f32>,
    // Strengths: retrace, thin, sharpen, unused.
    strength: vec4<f32>,
}

@group(0) @binding(0) var<uniform> p: Params;
// The source, as RGB bytes (3 a pixel), read four bytes at a time.
@group(0) @binding(1) var<storage, read> src: array<u32>;
// The source unpacked, one pixel a word.
@group(0) @binding(2) var<storage, read_write> img: array<u32>;
// Two output-sized buffers the passes write to in turn.
@group(0) @binding(3) var<storage, read_write> a: array<u32>;
@group(0) @binding(4) var<storage, read_write> b: array<u32>;

fn unpack(c: u32) -> vec4<f32> {
    return vec4<f32>(f32(c & 255u), f32((c >> 8u) & 255u), f32((c >> 16u) & 255u), f32(c >> 24u)) / 255.0;
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.299, 0.587, 0.114));
}

fn pack(c: vec3<f32>) -> u32 {
    let v = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    let q = vec4<u32>(round(vec4<f32>(v, luma(v)) * 255.0));
    return q.x | (q.y << 8u) | (q.z << 16u) | (q.w << 24u);
}

fn src_byte(i: u32) -> u32 {
    return (src[i >> 2u] >> ((i & 3u) * 8u)) & 255u;
}

// ---- 1. Unpack the RGB source --------------------------------------------

@compute @workgroup_size(16, 16)
fn unpack_source(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= p.iw || id.y >= p.ih) { return; }
    let i = id.y * p.iw + id.x;
    let c = vec3<f32>(f32(src_byte(i * 3u)), f32(src_byte(i * 3u + 1u)), f32(src_byte(i * 3u + 2u))) / 255.0;
    img[i] = pack(c);
}

// ---- 2. Upscale: bicubic, without ringing --------------------------------

fn texel(x: i32, y: i32) -> vec3<f32> {
    let cx = u32(clamp(x, 0, i32(p.iw) - 1));
    let cy = u32(clamp(y, 0, i32(p.ih) - 1));
    return unpack(img[cy * p.iw + cx]).rgb;
}

// Catmull-Rom weights for the four taps around t in [0, 1).
fn cubic(t: f32) -> vec4<f32> {
    let t2 = t * t;
    let t3 = t2 * t;
    return vec4<f32>(
        -0.5 * t3 + t2 - 0.5 * t,
        1.5 * t3 - 2.5 * t2 + 1.0,
        -1.5 * t3 + 2.0 * t2 + 0.5 * t,
        0.5 * t3 - 0.5 * t2,
    );
}

fn row(bx: i32, y: i32, wx: vec4<f32>) -> vec3<f32> {
    return texel(bx - 1, y) * wx.x + texel(bx, y) * wx.y + texel(bx + 1, y) * wx.z + texel(bx + 2, y) * wx.w;
}

@compute @workgroup_size(16, 16)
fn upscale(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= p.ow || id.y >= p.oh) { return; }
    let scale = vec2<f32>((p.rect.z - p.rect.x) / f32(p.ow), (p.rect.w - p.rect.y) / f32(p.oh));
    let pos = p.rect.xy + (vec2<f32>(id.xy) + 0.5) * scale - 0.5;
    let base = floor(pos);
    let f = pos - base;
    let wx = cubic(f.x);
    let wy = cubic(f.y);
    let bx = i32(base.x);
    let by = i32(base.y);
    // Written out in full: weights indexed in a loop would leave registers
    // for slow memory on most GPUs.
    let r0 = row(bx, by - 1, wx);
    let n0 = texel(bx, by);
    let n1 = texel(bx + 1, by);
    let n2 = texel(bx, by + 1);
    let n3 = texel(bx + 1, by + 1);
    let r1 = texel(bx - 1, by) * wx.x + n0 * wx.y + n1 * wx.z + texel(bx + 2, by) * wx.w;
    let r2 = texel(bx - 1, by + 1) * wx.x + n2 * wx.y + n3 * wx.z + texel(bx + 2, by + 1) * wx.w;
    let r3 = row(bx, by + 2, wx);
    let c = r0 * wy.x + r1 * wy.y + r2 * wy.z + r3 * wy.w;
    // Held to the four nearest source pixels' range, so edges get no halo.
    a[id.y * p.ow + id.x] = pack(clamp(c, min(min(n0, n1), min(n2, n3)), max(max(n0, n1), max(n2, n3))));
}

// ---- 3. Refine line art: gradient, retrace, thin --------------------------
//
// Separate passes, each reading its neighbours from the last one's
// output: on the GPUs tried, this beat doing all three in one pass through
// workgroup memory by four times (the barriers cost more than the reads).

fn idx(x: i32, y: i32) -> u32 {
    return u32(clamp(y, 0, i32(p.oh) - 1)) * p.ow + u32(clamp(x, 0, i32(p.ow) - 1));
}

// The luma stored with a pixel, without unpacking the rest.
fn la(x: i32, y: i32) -> f32 {
    return f32(a[idx(x, y)] >> 24u) / 255.0;
}

fn lb(x: i32, y: i32) -> f32 {
    return f32(b[idx(x, y)] >> 24u) / 255.0;
}

@group(0) @binding(5) var<storage, read_write> grad: array<f32>;

// Sobel on luma, from the upscaled picture (a) into grad.
@compute @workgroup_size(16, 16)
fn gradient(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= p.ow || id.y >= p.oh) { return; }
    let x = i32(id.x);
    let y = i32(id.y);
    let tl = la(x - 1, y - 1); let t = la(x, y - 1); let tr = la(x + 1, y - 1);
    let l = la(x - 1, y);                          let r = la(x + 1, y);
    let bl = la(x - 1, y + 1); let bo = la(x, y + 1); let br = la(x + 1, y + 1);
    let gx = (tr + 2.0 * r + br) - (tl + 2.0 * l + bl);
    let gy = (bl + 2.0 * bo + br) - (tl + 2.0 * t + tr);
    grad[id.y * p.ow + id.x] = sqrt(gx * gx + gy * gy);
}

// Retrace, a into b: an enlarged edge is a ramp a few pixels wide. A
// pixel on the ramp has a steeper gradient than its neighbours off it, so
// it takes the colour of the neighbour with the flattest gradient, and the
// ramp collapses into a clean line again. Flat areas are left alone.
@compute @workgroup_size(16, 16)
fn retrace(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= p.ow || id.y >= p.oh) { return; }
    let x = i32(id.x);
    let y = i32(id.y);
    let i = id.y * p.ow + id.x;
    let here = grad[i];
    var best = here;
    var at = i;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let n = idx(x + dx, y + dy);
            let g = grad[n];
            if (g < best) {
                best = g;
                at = n;
            }
        }
    }
    let ramp = clamp((here - best) * 4.0, 0.0, 1.0) * p.strength.x;
    var result = a[i];
    if (ramp > 0.0) {
        result = pack(mix(unpack(result).rgb, unpack(a[at]).rgb, ramp));
    }
    b[i] = result;
}

// Thin, b into a: enlarging spreads a dark line's edges into grey. A pixel
// between a clearly darker side and a clearly lighter one moves toward the
// lighter side, so lines keep their weight.
@compute @workgroup_size(16, 16)
fn thin(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= p.ow || id.y >= p.oh) { return; }
    let x = i32(id.x);
    let y = i32(id.y);
    let i = id.y * p.ow + id.x;
    let here = f32(b[i] >> 24u) / 255.0;
    var light = here;
    var dark = here;
    var lightest = i;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let n = idx(x + dx, y + dy);
            let v = f32(b[n] >> 24u) / 255.0;
            if (v > light) { light = v; lightest = n; }
            dark = min(dark, v);
        }
    }
    let across = clamp((light - dark) * 3.0 - 0.3, 0.0, 1.0);
    let middle = 1.0 - abs((here - dark) / max(light - dark, 0.001) - 0.5) * 2.0;
    let amount = across * max(middle, 0.0) * p.strength.y;
    var result = b[i];
    if (amount > 0.0) {
        result = pack(mix(unpack(result).rgb, unpack(b[lightest]).rgb, amount));
    }
    a[i] = result;
}

// ---- 4. Sharpen, adapting to contrast, as 0RGB -----------------------------
//
// Like AMD's RCAS: a cross of neighbours sets how much a pixel may be
// sharpened, so busy areas aren't over-sharpened, and the result is held
// to the neighbours' range (no halos).

fn sharpened(c: vec3<f32>, n: vec3<f32>, s: vec3<f32>, w: vec3<f32>, e: vec3<f32>) -> u32 {
    let lo = min(c, min(min(n, s), min(w, e)));
    let hi = max(c, max(max(n, s), max(w, e)));
    // Room to sharpen before clipping, per channel.
    let room = min(lo, 1.0 - hi) / max(hi, vec3<f32>(0.001));
    let amount = sqrt(clamp(min(room.r, min(room.g, room.b)), 0.0, 1.0)) * p.strength.z;
    let o = clamp(c + (4.0 * c - (n + s + w + e)) * amount * 0.25, lo, hi);
    let v = vec3<u32>(round(clamp(o, vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0));
    return (v.x << 16u) | (v.y << 8u) | v.z;
}

fn a_at(x: i32, y: i32) -> vec3<f32> {
    let cx = u32(clamp(x, 0, i32(p.ow) - 1));
    let cy = u32(clamp(y, 0, i32(p.oh) - 1));
    return unpack(a[cy * p.ow + cx]).rgb;
}

@compute @workgroup_size(16, 16)
fn sharpen(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= p.ow || id.y >= p.oh) { return; }
    let x = i32(id.x);
    let y = i32(id.y);
    b[id.y * p.ow + id.x] = sharpened(a_at(x, y), a_at(x, y - 1), a_at(x, y + 1), a_at(x - 1, y), a_at(x + 1, y));
}
