// Spot removal (`lightcraft_pipeline::spots::apply`), one spot at a time, on the image in place.
// Bindings: img (rgb, read-write), pts (f32 data: see each kernel), al / sp / df / wt (work buffers
// over the spot's box: alpha, source patch, masked difference, weights).
//
// Parameters of spot_prep / spot_apply: W, H, BX, BY, BW, BH, NPTS, R, INNER, OPACITY, OFFX, OFFY,
// HEAL. `pts`: the stroke's points (x, y; output pixels).

// `f32::MAX` exactly ("doesn't fit", as the CPU's score; a decimal literal may round below it,
// and naga can't evaluate bitcast in a const).
fn f32_max() -> f32 {
    return bitcast<f32>(0x7f7fffffu);
}

// `Rgb32f::get_clamped`.
fn px_at(x: i32, y: i32) -> vec3<f32> {
    let w = i32(pu(0u));
    let h = i32(pu(1u));
    let i = u32(clamp(y, 0, h - 1) * w + clamp(x, 0, w - 1));
    return vec3<f32>(img[3u * i], img[3u * i + 1u], img[3u * i + 2u]);
}

// `Rgb32f::sample_bilinear` (pixel centres at +0.5).
fn bilinear(x: f32, y: f32) -> vec3<f32> {
    let fx = x - 0.5;
    let fy = y - 0.5;
    let x0 = floor(fx);
    let y0 = floor(fy);
    let tx = fx - x0;
    let ty = fy - y0;
    let ix = i32(x0);
    let iy = i32(y0);
    let a = px_at(ix, iy);
    let b = px_at(ix + 1, iy);
    let c = px_at(ix, iy + 1);
    let d = px_at(ix + 1, iy + 1);
    let top = a + (b - a) * tx;
    let bot = c + (d - c) * tx;
    return top + (bot - top) * ty;
}

// `spots::smooth`, defined for e1 ≤ e0 as the step at e1 (no feather).
fn spot_smooth(e0: f32, e1: f32, x: f32) -> f32 {
    if (e1 <= e0) {
        return select(0.0, 1.0, x >= e1);
    }
    return sstep(e0, e1, x);
}

// Alpha, source patch and (Heal) the difference outside the stroke, over the box.
@compute @workgroup_size(16, 16)
fn spot_prep(@builtin(global_invocation_id) gid: vec3<u32>) {
    let bw = pu(4u);
    let bh = pu(5u);
    if (gid.x >= bw || gid.y >= bh) {
        return;
    }
    let i = gid.y * bw + gid.x;
    let gx = pu(2u) + gid.x;
    let gy = pu(3u) + gid.y;
    let px = f32(gx) + 0.5;
    let py = f32(gy) + 0.5;
    var d = f32_max();
    for (var k = 0u; k < pu(6u); k++) {
        let ex = px - pts[2u * k];
        let ey = py - pts[2u * k + 1u];
        d = min(d, sqrt(ex * ex + ey * ey));
    }
    let alpha = (1.0 - spot_smooth(pf(8u), pf(7u), d)) * pf(9u);
    al[i] = alpha;
    let s = bilinear(px + pf(10u), py + pf(11u));
    sp[3u * i] = s.x;
    sp[3u * i + 1u] = s.y;
    sp[3u * i + 2u] = s.z;
    let gi = gy * pu(0u) + gx;
    let t = vec3<f32>(img[3u * gi], img[3u * gi + 1u], img[3u * gi + 2u]);
    let outside = select(0.0, 1.0, alpha < 0.02);
    let m = (t - s) * outside;
    df[3u * i] = m.x;
    df[3u * i + 1u] = m.y;
    df[3u * i + 2u] = m.z;
    wt[i] = outside;
}

// Blend the (healed) patch into the image (df: Heal's tone correction, the membrane).
@compute @workgroup_size(16, 16)
fn spot_apply(@builtin(global_invocation_id) gid: vec3<u32>) {
    let bw = pu(4u);
    let bh = pu(5u);
    if (gid.x >= bw || gid.y >= bh) {
        return;
    }
    let i = gid.y * bw + gid.x;
    let a = al[i];
    if (a <= 0.0) {
        return;
    }
    var v = vec3<f32>(sp[3u * i], sp[3u * i + 1u], sp[3u * i + 2u]);
    if (pu(12u) != 0u) {
        v = v + vec3<f32>(df[3u * i], df[3u * i + 1u], df[3u * i + 2u]);
    }
    let gi = (pu(3u) + gid.y) * pu(0u) + pu(2u) + gid.x;
    let t = vec3<f32>(img[3u * gi], img[3u * gi + 1u], img[3u * gi + 2u]);
    let o = max(t + (v - t) * a, vec3<f32>(0.0));
    img[3u * gi] = o.x;
    img[3u * gi + 1u] = o.y;
    img[3u * gi + 2u] = o.z;
}

// `spots::ring_stats`: sample k (spoke-major, 3 radii each) of the rings around c, or a NaN-free
// flag in .w = 0 when it falls outside the image.
fn ring_sample(c: vec2<f32>, k: u32, radii: u32) -> vec4<f32> {
    let spoke = k / 3u;
    let rr = pf(radii + k % 3u);
    let o = 2u * NCAND + 2u * spoke;
    let x = c.x + pts[o] * rr;
    let y = c.y + pts[o + 1u] * rr;
    if (x < 0.0 || y < 0.0 || x >= f32(pu(0u)) || y >= f32(pu(1u))) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(bilinear(x, y), 1.0);
}

fn ring_fits(c: vec2<f32>, radii: u32) -> bool {
    for (var k = 0u; k < 72u; k++) {
        if (ring_sample(c, k, radii).w == 0.0) {
            return false;
        }
    }
    return true;
}

fn sq(v: f32) -> f32 {
    return sqrt(max(v, 0.0));
}

// `spots::score` of every candidate source (one invocation each) into `al`.
// Parameters: W, H, TX, TY, then the surroundings' radii (3) and the inside's (3).
// `pts`: NCAND candidate offsets (x, y), then the 24 spoke directions (cos, sin).
@compute @workgroup_size(64)
fn spot_score(@builtin(global_invocation_id) gid: vec3<u32>) {
    let j = gid.x;
    if (j >= NCAND) {
        return;
    }
    let tgt = vec2<f32>(pf(2u), pf(3u));
    let src = tgt + vec2<f32>(pts[2u * j], pts[2u * j + 1u]);
    if (!ring_fits(tgt, 4u) || !ring_fits(src, 4u)) {
        al[j] = f32_max();
        return;
    }
    var s = 0.0;
    for (var k = 0u; k < 72u; k++) {
        let p = ring_sample(tgt, k, 4u).xyz;
        let q = ring_sample(src, k, 4u).xyz;
        for (var c = 0u; c < 3u; c++) {
            let u = sq(p[c]);
            let v = sq(q[c]);
            s += (u - v) * (u - v);
        }
    }
    if (ring_fits(src, 7u)) {
        var mean_a = 0.0;
        for (var k = 0u; k < 72u; k++) {
            mean_a += sq(ring_sample(tgt, k, 4u).y);
        }
        mean_a = mean_a / 72.0;
        var var_ = 0.0;
        for (var k = 0u; k < 72u; k++) {
            let e = sq(ring_sample(src, k, 7u).y) - mean_a;
            var_ += e * e;
        }
        var_ = var_ / 72.0;
        s += var_ * 72.0 * 0.5;
    }
    al[j] = s;
}

// Heal's membrane (`spots::membrane`), cascadic multigrid with red-black SOR, level by level.

// One level coarser: a coarse pixel is known when any of its children is, with their mean; unknown
// ones start at 0. Parameters: FW, FH, CW, CH. df / wt: the fine values (3 per pixel) and known
// flags; sp / al: the coarse ones.
@compute @workgroup_size(16, 16)
fn mem_restrict(@builtin(global_invocation_id) gid: vec3<u32>) {
    let fw = pu(0u);
    let fh = pu(1u);
    let cw = pu(2u);
    let ch = pu(3u);
    if (gid.x >= cw || gid.y >= ch) {
        return;
    }
    var s = vec3<f32>(0.0);
    var n = 0.0;
    for (var dy = 0u; dy < 2u; dy++) {
        for (var dx = 0u; dx < 2u; dx++) {
            let x = gid.x * 2u + dx;
            let y = gid.y * 2u + dy;
            if (x < fw && y < fh) {
                let j = y * fw + x;
                if (wt[j] > 0.5) {
                    s = s + vec3<f32>(df[3u * j], df[3u * j + 1u], df[3u * j + 2u]);
                    n = n + 1.0;
                }
            }
        }
    }
    let i = gid.y * cw + gid.x;
    var v = vec3<f32>(0.0);
    var k = 0.0;
    if (n > 0.0) {
        v = s / n;
        k = 1.0;
    }
    sp[3u * i] = v.x;
    sp[3u * i + 1u] = v.y;
    sp[3u * i + 2u] = v.z;
    al[i] = k;
}

fn coarse_at(x: u32, y: u32, cw: u32) -> vec3<f32> {
    let i = y * cw + x;
    return vec3<f32>(sp[3u * i], sp[3u * i + 1u], sp[3u * i + 2u]);
}

// The unknown pixels of a level from the coarser solution (bilinear, pixel centres). Parameters:
// FW, FH, CW, CH. sp: the coarse values; df / wt: the fine values and known flags.
@compute @workgroup_size(16, 16)
fn mem_prolong(@builtin(global_invocation_id) gid: vec3<u32>) {
    let fw = pu(0u);
    let fh = pu(1u);
    let cw = pu(2u);
    let ch = pu(3u);
    if (gid.x >= fw || gid.y >= fh) {
        return;
    }
    let i = gid.y * fw + gid.x;
    if (wt[i] > 0.5) {
        return;
    }
    let fy = clamp((f32(gid.y) + 0.5) / 2.0 - 0.5, 0.0, f32(ch - 1u));
    let y0 = u32(floor(fy));
    let y1 = min(y0 + 1u, ch - 1u);
    let ty = fy - f32(y0);
    let fx = clamp((f32(gid.x) + 0.5) / 2.0 - 0.5, 0.0, f32(cw - 1u));
    let x0 = u32(floor(fx));
    let x1 = min(x0 + 1u, cw - 1u);
    let tx = fx - f32(x0);
    let a = coarse_at(x0, y0, cw);
    let b = coarse_at(x1, y0, cw);
    let c = coarse_at(x0, y1, cw);
    let d = coarse_at(x1, y1, cw);
    let top = a + (b - a) * tx;
    let bot = c + (d - c) * tx;
    let v = top + (bot - top) * ty;
    df[3u * i] = v.x;
    df[3u * i + 1u] = v.y;
    df[3u * i + 2u] = v.z;
}

fn level_at(j: u32) -> vec3<f32> {
    return vec3<f32>(df[3u * j], df[3u * j + 1u], df[3u * j + 2u]);
}

// Half an SOR sweep in place: the unknown pixels with (x + y) % 2 == PARITY, from their neighbours
// (the other half). Parameters: W, H, PARITY, OMEGA. df / wt: the values and known flags.
@compute @workgroup_size(16, 16)
fn mem_sor(@builtin(global_invocation_id) gid: vec3<u32>) {
    let w = pu(0u);
    let h = pu(1u);
    if (gid.x >= w || gid.y >= h || (gid.x + gid.y) % 2u != pu(2u)) {
        return;
    }
    let i = gid.y * w + gid.x;
    if (wt[i] > 0.5) {
        return;
    }
    var s = vec3<f32>(0.0);
    var n = 0.0;
    if (gid.x > 0u) {
        s = s + level_at(i - 1u);
        n = n + 1.0;
    }
    if (gid.x + 1u < w) {
        s = s + level_at(i + 1u);
        n = n + 1.0;
    }
    if (gid.y > 0u) {
        s = s + level_at(i - w);
        n = n + 1.0;
    }
    if (gid.y + 1u < h) {
        s = s + level_at(i + w);
        n = n + 1.0;
    }
    if (n > 0.0) {
        let old = level_at(i);
        let v = old + pf(3u) * (s / n - old);
        df[3u * i] = v.x;
        df[3u * i + 1u] = v.y;
        df[3u * i + 2u] = v.z;
    }
}
