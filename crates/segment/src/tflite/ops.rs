//! The operator kernels of the TFLite interpreter, on NHWC `f32` buffers.
//!
//! Semantics follow the TensorFlow Lite builtin operators as documented in TensorFlow
//! (`tensorflow/lite/schema/schema.fbs` option fields; `tf.nn` / `tf.image` behaviour for padding,
//! resizing and activations), Apache-2.0, Copyright The TensorFlow Authors; written from that
//! behaviour by the LightCraft contributors in 2026, no code copied.
//!
//! Every kernel checks the lengths of its inputs against the shapes validated at load before it
//! touches them; inside the loops, offsets come from those shapes.

use candle_core::{CpuStorage, Device, Storage, Tensor};
use rayon::prelude::*;

use super::bad;
use crate::Result;

/// A fused activation (`ActivationFunctionType`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Act {
    None,
    Relu,
    ReluN1To1,
    Relu6,
}

impl Act {
    pub fn from_code(code: i8) -> Result<Self> {
        Ok(match code {
            0 => Act::None,
            1 => Act::Relu,
            2 => Act::ReluN1To1,
            3 => Act::Relu6,
            c => return Err(bad(&format!("fused activation {c} is not supported (NONE, RELU, RELU_N1_TO_1, RELU6 are)"))),
        })
    }

    #[inline]
    pub fn apply(self, v: f32) -> f32 {
        match self {
            Act::None => v,
            Act::Relu => v.max(0.0),
            Act::ReluN1To1 => v.clamp(-1.0, 1.0),
            Act::Relu6 => v.clamp(0.0, 6.0),
        }
    }

    fn apply_all(self, v: &mut [f32]) {
        if self != Act::None {
            v.iter_mut().for_each(|x| *x = self.apply(*x));
        }
    }
}

/// Rows of output per parallel task in element-wise kernels.
const CHUNK: usize = 1 << 14;

fn check(what: &str, v: &[f32], n: usize) -> Result<()> {
    if v.len() != n {
        return Err(bad(&format!("{what}: {} values where {n} were expected", v.len())));
    }
    Ok(())
}

/// Padding before the data for SAME/VALID on one axis (TFLite's rule: half of the total, the
/// extra pixel after).
pub fn pad_before(same: bool, input: usize, kernel: usize, stride: usize, dilation: usize) -> (usize, usize) {
    let eff = kernel.saturating_sub(1).saturating_mul(dilation).saturating_add(1);
    let stride = stride.max(1);
    let out = if same { input.div_ceil(stride) } else { input.saturating_sub(eff) / stride + 1 };
    let total = out.saturating_sub(1).saturating_mul(stride).saturating_add(eff).saturating_sub(input);
    (out, total / 2)
}

/// `a (m × k) · w (k × n)` on candle's CPU matrix product, then `act(· + bias)`.
fn gemm(a: Vec<f32>, m: usize, k: usize, w: &Tensor, bias: Option<&[f32]>, act: Act) -> Result<Vec<f32>> {
    let n = w.dims().get(1).copied().unwrap_or(0);
    if m == 0 || n == 0 {
        return Ok(vec![0.0; m * n]);
    }
    let c = Tensor::from_vec(a, (m, k), &Device::Cpu)?.matmul(w)?;
    let (storage, layout) = c.storage_and_layout();
    let Storage::Cpu(CpuStorage::F32(v)) = &*storage else {
        return Err(bad("matrix product: unexpected storage"));
    };
    let v = layout.contiguous_offsets().and_then(|(s, e)| v.get(s..e)).ok_or_else(|| bad("matrix product: layout"))?;
    check("matrix product", v, m * n)?;
    let mut out = vec![0f32; m * n];
    let rows = (CHUNK / n).max(1) * n;
    out.par_chunks_mut(rows).zip(v.par_chunks(rows)).for_each(|(o, c)| match bias {
        Some(b) => {
            for (orow, crow) in o.chunks_exact_mut(n).zip(c.chunks_exact(n)) {
                for ((o, c), b) in orow.iter_mut().zip(crow).zip(b) {
                    *o = act.apply(c + b);
                }
            }
        }
        None => {
            for (o, c) in o.iter_mut().zip(c) {
                *o = act.apply(*c);
            }
        }
    });
    Ok(out)
}

/// The geometry of a 2-D window operation (convolution, pooling) on NHWC data.
#[derive(Clone, Debug)]
pub struct Window {
    pub n: usize,
    pub h: usize,
    pub w: usize,
    pub c: usize,
    pub oh: usize,
    pub ow: usize,
    pub kh: usize,
    pub kw: usize,
    pub sh: usize,
    pub sw: usize,
    pub dh: usize,
    pub dw: usize,
    pub pt: usize,
    pub pl: usize,
}

impl Window {
    pub fn new(same: bool, [n, h, w, c]: [usize; 4], (kh, kw): (usize, usize), (sh, sw): (usize, usize), (dh, dw): (usize, usize)) -> Self {
        let (oh, pt) = pad_before(same, h, kh, sh, dh);
        let (ow, pl) = pad_before(same, w, kw, sw, dw);
        Self { n, h, w, c, oh, ow, kh, kw, sh, sw, dh, dw, pt, pl }
    }

    /// Input row/column of kernel tap `k` for output `o` on an axis, if inside the image.
    #[inline]
    fn tap(o: usize, k: usize, stride: usize, dil: usize, pad: usize, size: usize) -> Option<usize> {
        (o * stride + k * dil).checked_sub(pad).filter(|&i| i < size)
    }

    fn in_len(&self) -> usize {
        self.n * self.h * self.w * self.c
    }
}

/// CONV_2D: im2col + matrix product. `w` is the pre-transposed `(kh·kw·c) × o` weight matrix.
pub struct Conv {
    pub g: Window,
    pub o: usize,
    pub w: Tensor,
    pub bias: Vec<f32>,
    pub act: Act,
}

impl Conv {
    pub fn out_shape(&self) -> [usize; 4] {
        [self.g.n, self.g.oh, self.g.ow, self.o]
    }

    pub fn run(&self, x: &[f32]) -> Result<Vec<f32>> {
        let g = &self.g;
        check("CONV_2D input", x, g.in_len())?;
        let k = g.kh * g.kw * g.c;
        let m = g.n * g.oh * g.ow;
        let pointwise = g.kh == 1 && g.kw == 1 && g.sh == 1 && g.sw == 1 && g.pt == 0 && g.pl == 0;
        let col = if pointwise { x.to_vec() } else { im2col(g, x) };
        gemm(col, m, k, &self.w, Some(&self.bias), self.act)
    }
}

fn im2col(g: &Window, x: &[f32]) -> Vec<f32> {
    let k = g.kh * g.kw * g.c;
    let mut col = vec![0f32; g.n * g.oh * g.ow * k];
    if k == 0 || g.ow == 0 {
        return col;
    }
    col.par_chunks_mut(g.ow * k).enumerate().for_each(|(row, dst)| {
        let (b, oy) = (row / g.oh, row % g.oh);
        for (ox, dst) in dst.chunks_exact_mut(k).enumerate() {
            for ky in 0..g.kh {
                let Some(iy) = Window::tap(oy, ky, g.sh, g.dh, g.pt, g.h) else { continue };
                for kx in 0..g.kw {
                    let Some(ix) = Window::tap(ox, kx, g.sw, g.dw, g.pl, g.w) else { continue };
                    let src = ((b * g.h + iy) * g.w + ix) * g.c;
                    let at = (ky * g.kw + kx) * g.c;
                    if let (Some(d), Some(s)) = (dst.get_mut(at..at + g.c), x.get(src..src + g.c)) {
                        d.copy_from_slice(s);
                    }
                }
            }
        }
    });
    col
}

/// DEPTHWISE_CONV_2D: direct loops. `w` is `kh × kw × (c·mult)`, as stored.
pub struct Depthwise {
    pub g: Window,
    pub mult: usize,
    pub w: Vec<f32>,
    pub bias: Vec<f32>,
    pub act: Act,
}

impl Depthwise {
    pub fn out_shape(&self) -> [usize; 4] {
        [self.g.n, self.g.oh, self.g.ow, self.g.c * self.mult]
    }

    pub fn run(&self, x: &[f32]) -> Result<Vec<f32>> {
        let g = &self.g;
        check("DEPTHWISE_CONV_2D input", x, g.in_len())?;
        let oc = g.c * self.mult;
        check("DEPTHWISE_CONV_2D weights", &self.w, g.kh * g.kw * oc)?;
        check("DEPTHWISE_CONV_2D bias", &self.bias, oc)?;
        let mut out = vec![0f32; g.n * g.oh * g.ow * oc];
        if oc == 0 || g.ow == 0 {
            return Ok(out);
        }
        out.par_chunks_mut(g.ow * oc).enumerate().for_each(|(row, dst)| {
            let (b, oy) = (row / g.oh, row % g.oh);
            for (ox, acc) in dst.chunks_exact_mut(oc).enumerate() {
                acc.copy_from_slice(&self.bias);
                for ky in 0..g.kh {
                    let Some(iy) = Window::tap(oy, ky, g.sh, g.dh, g.pt, g.h) else { continue };
                    for kx in 0..g.kw {
                        let Some(ix) = Window::tap(ox, kx, g.sw, g.dw, g.pl, g.w) else { continue };
                        let src = ((b * g.h + iy) * g.w + ix) * g.c;
                        let wk = (ky * g.kw + kx) * oc;
                        let (Some(inp), Some(wk)) = (x.get(src..src + g.c), self.w.get(wk..wk + oc)) else { continue };
                        if self.mult == 1 {
                            for ((a, v), w) in acc.iter_mut().zip(inp).zip(wk) {
                                *a += v * w;
                            }
                        } else {
                            for ((a, w), i) in acc.chunks_exact_mut(self.mult).zip(wk.chunks_exact(self.mult)).zip(inp) {
                                for (a, w) in a.iter_mut().zip(w) {
                                    *a += i * w;
                                }
                            }
                        }
                    }
                }
                self.act.apply_all(acc);
            }
        });
        Ok(out)
    }
}

/// TRANSPOSE_CONV: `input · w` gives every input pixel's `kh × kw × o` contribution, gathered into
/// the output. `w` is the `c × (kh·kw·o)` matrix. `g` describes the *forward* convolution from
/// the output (`oh × ow`) back to the input (`h × w`): `g.h, g.w` are the output size here.
pub struct TransposeConv {
    pub n: usize,
    pub h: usize,
    pub w: usize,
    pub c: usize,
    pub oh: usize,
    pub ow: usize,
    pub o: usize,
    pub kh: usize,
    pub kw: usize,
    pub sh: usize,
    pub sw: usize,
    pub pt: usize,
    pub pl: usize,
    pub weights: Tensor,
    pub bias: Vec<f32>,
    pub act: Act,
}

impl TransposeConv {
    pub fn out_shape(&self) -> [usize; 4] {
        [self.n, self.oh, self.ow, self.o]
    }

    pub fn run(&self, x: &[f32]) -> Result<Vec<f32>> {
        check("TRANSPOSE_CONV input", x, self.n * self.h * self.w * self.c)?;
        check("TRANSPOSE_CONV bias", &self.bias, self.o)?;
        let kko = self.kh * self.kw * self.o;
        let cols = gemm(x.to_vec(), self.n * self.h * self.w, self.c, &self.weights, None, Act::None)?;
        check("TRANSPOSE_CONV product", &cols, self.n * self.h * self.w * kko)?;
        let mut out = vec![0f32; self.n * self.oh * self.ow * self.o];
        if self.o == 0 || self.ow == 0 {
            return Ok(out);
        }
        // output pixel (oy, ox) receives input (y, x) through tap (ky, kx) when y·s + k − pad = o
        let src = |o: usize, k: usize, s: usize, pad: usize, size: usize| -> Option<usize> {
            let t = (o + pad).checked_sub(k)?;
            (t % s == 0 && t / s < size).then_some(t / s)
        };
        out.par_chunks_mut(self.ow * self.o).enumerate().for_each(|(row, dst)| {
            let (b, oy) = (row / self.oh, row % self.oh);
            for (ox, acc) in dst.chunks_exact_mut(self.o).enumerate() {
                acc.copy_from_slice(&self.bias);
                for ky in 0..self.kh {
                    let Some(y) = src(oy, ky, self.sh, self.pt, self.h) else { continue };
                    for kx in 0..self.kw {
                        let Some(x) = src(ox, kx, self.sw, self.pl, self.w) else { continue };
                        let at = ((b * self.h + y) * self.w + x) * kko + (ky * self.kw + kx) * self.o;
                        let Some(c) = cols.get(at..at + self.o) else { continue };
                        for (a, c) in acc.iter_mut().zip(c) {
                            *a += c;
                        }
                    }
                }
                self.act.apply_all(acc);
            }
        });
        Ok(out)
    }
}

/// MAX_POOL_2D (padding cells excluded).
pub struct MaxPool {
    pub g: Window,
    pub act: Act,
}

impl MaxPool {
    pub fn out_shape(&self) -> [usize; 4] {
        [self.g.n, self.g.oh, self.g.ow, self.g.c]
    }

    pub fn run(&self, x: &[f32]) -> Result<Vec<f32>> {
        let g = &self.g;
        check("MAX_POOL_2D input", x, g.in_len())?;
        let mut out = vec![0f32; g.n * g.oh * g.ow * g.c];
        if g.c == 0 || g.ow == 0 {
            return Ok(out);
        }
        out.par_chunks_mut(g.ow * g.c).enumerate().for_each(|(row, dst)| {
            let (b, oy) = (row / g.oh, row % g.oh);
            for (ox, acc) in dst.chunks_exact_mut(g.c).enumerate() {
                acc.fill(f32::NEG_INFINITY);
                for ky in 0..g.kh {
                    let Some(iy) = Window::tap(oy, ky, g.sh, 1, g.pt, g.h) else { continue };
                    for kx in 0..g.kw {
                        let Some(ix) = Window::tap(ox, kx, g.sw, 1, g.pl, g.w) else { continue };
                        let src = ((b * g.h + iy) * g.w + ix) * g.c;
                        let Some(inp) = x.get(src..src + g.c) else { continue };
                        for (a, v) in acc.iter_mut().zip(inp) {
                            *a = a.max(*v);
                        }
                    }
                }
                self.act.apply_all(acc);
            }
        });
        Ok(out)
    }
}

/// Element-wise binary operations with NumPy broadcasting (over 4-D shapes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BinOp {
    Add,
    Mul,
    /// `x ≥ 0 ? x : x · alpha` (PRELU; the second operand is alpha).
    Prelu,
}

pub struct Binary {
    pub op: BinOp,
    pub a: [usize; 4],
    pub b: [usize; 4],
    pub out: [usize; 4],
    pub act: Act,
}

/// The broadcast shape of two 4-D shapes.
pub fn broadcast(a: [usize; 4], b: [usize; 4]) -> Result<[usize; 4]> {
    let mut o = [0; 4];
    for i in 0..4 {
        o[i] = match (a[i], b[i]) {
            (x, y) if x == y => x,
            (1, y) => y,
            (x, 1) => x,
            _ => return Err(bad(&format!("shapes {a:?} and {b:?} don't broadcast"))),
        };
    }
    Ok(o)
}

fn strides(d: [usize; 4]) -> [usize; 4] {
    [d[1] * d[2] * d[3], d[2] * d[3], d[3], 1]
}

/// Strides that repeat broadcast (size-1) axes.
fn bstrides(d: [usize; 4]) -> [usize; 4] {
    let s = strides(d);
    std::array::from_fn(|i| if d[i] == 1 { 0 } else { s[i] })
}

impl Binary {
    #[inline]
    fn f(&self, x: f32, y: f32) -> f32 {
        let v = match self.op {
            BinOp::Add => x + y,
            BinOp::Mul => x * y,
            BinOp::Prelu => {
                if x >= 0.0 {
                    x
                } else {
                    x * y
                }
            }
        };
        self.act.apply(v)
    }

    pub fn run(&self, a: &[f32], b: &[f32]) -> Result<Vec<f32>> {
        check("binary operand", a, self.a.iter().product())?;
        check("binary operand", b, self.b.iter().product())?;
        let n: usize = self.out.iter().product();
        let mut out = vec![0f32; n];
        if self.a == self.b {
            out.par_chunks_mut(CHUNK).zip(a.par_chunks(CHUNK).zip(b.par_chunks(CHUNK))).for_each(|(o, (a, b))| {
                for ((o, a), b) in o.iter_mut().zip(a).zip(b) {
                    *o = self.f(*a, *b);
                }
            });
            return Ok(out);
        }
        let d3 = self.out[3];
        if d3 == 0 {
            return Ok(out);
        }
        let (sa, sb) = (bstrides(self.a), bstrides(self.b));
        let (o1, o2) = (self.out[1], self.out[2]);
        out.par_chunks_mut(d3).enumerate().for_each(|(row, dst)| {
            let (i0, i1, i2) = (row / (o1 * o2), row / o2 % o1, row % o2);
            let ba = i0 * sa[0] + i1 * sa[1] + i2 * sa[2];
            let bb = i0 * sb[0] + i1 * sb[1] + i2 * sb[2];
            for (i3, o) in dst.iter_mut().enumerate() {
                if let (Some(x), Some(y)) = (a.get(ba + i3 * sa[3]), b.get(bb + i3 * sb[3])) {
                    *o = self.f(*x, *y);
                }
            }
        });
        Ok(out)
    }
}

/// Element-wise unary operations.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Unary {
    Act(Act),
    Logistic,
}

impl Unary {
    pub fn run(self, x: &[f32]) -> Vec<f32> {
        let mut out = x.to_vec();
        out.par_chunks_mut(CHUNK).for_each(|o| match self {
            Unary::Act(a) => a.apply_all(o),
            Unary::Logistic => o.iter_mut().for_each(|v| *v = 1.0 / (1.0 + (-*v).exp())),
        });
        out
    }
}

/// PAD with zeros (4-D, non-negative paddings).
pub struct Pad {
    pub d: [usize; 4],
    pub before: [usize; 4],
    pub out: [usize; 4],
}

impl Pad {
    pub fn run(&self, x: &[f32]) -> Result<Vec<f32>> {
        check("PAD input", x, self.d.iter().product())?;
        let mut out = vec![0f32; self.out.iter().product()];
        let so = strides(self.out);
        let d = self.d;
        if d[3] == 0 {
            return Ok(out);
        }
        for (row, src) in x.chunks_exact(d[3]).enumerate() {
            let (i0, i1, i2) = (row / (d[1] * d[2]), row / d[2] % d[1], row % d[2]);
            let at = (i0 + self.before[0]) * so[0] + (i1 + self.before[1]) * so[1] + (i2 + self.before[2]) * so[2] + self.before[3];
            if let Some(dst) = out.get_mut(at..at + d[3]) {
                dst.copy_from_slice(src);
            }
        }
        Ok(out)
    }
}

/// TRANSPOSE (4-D): output axis `j` is input axis `perm[j]`.
pub struct Transpose {
    pub d: [usize; 4],
    pub perm: [usize; 4],
}

impl Transpose {
    pub fn out_shape(&self) -> [usize; 4] {
        self.perm.map(|p| self.d.get(p).copied().unwrap_or(0))
    }

    pub fn run(&self, x: &[f32]) -> Result<Vec<f32>> {
        check("TRANSPOSE input", x, self.d.iter().product())?;
        let od = self.out_shape();
        let si = strides(self.d);
        let s: [usize; 4] = self.perm.map(|p| si.get(p).copied().unwrap_or(0));
        let mut out = vec![0f32; od.iter().product()];
        if od[3] == 0 {
            return Ok(out);
        }
        out.par_chunks_mut(od[3]).enumerate().for_each(|(row, dst)| {
            let (i0, i1, i2) = (row / (od[1] * od[2]), row / od[2] % od[1], row % od[2]);
            let base = i0 * s[0] + i1 * s[1] + i2 * s[2];
            for (i3, o) in dst.iter_mut().enumerate() {
                if let Some(v) = x.get(base + i3 * s[3]) {
                    *o = *v;
                }
            }
        });
        Ok(out)
    }
}

/// SOFTMAX over the last axis.
pub fn softmax(x: &[f32], inner: usize, beta: f32) -> Vec<f32> {
    let mut out = x.to_vec();
    if inner == 0 {
        return out;
    }
    out.par_chunks_mut(inner).for_each(|row| {
        let m = row.iter().fold(f32::NEG_INFINITY, |m, v| m.max(*v));
        let mut sum = 0f32;
        for v in row.iter_mut() {
            *v = ((*v - m) * beta).exp();
            sum += *v;
        }
        let inv = 1.0 / sum;
        row.iter_mut().for_each(|v| *v *= inv);
    });
    out
}

/// SUM over the axes flagged in `reduce` (4-D); the output keeps size-1 axes in their place.
pub struct Sum {
    pub d: [usize; 4],
    pub reduce: [bool; 4],
}

impl Sum {
    pub fn out_len(&self) -> usize {
        (0..4).map(|i| if self.reduce[i] { 1 } else { self.d[i] }).product()
    }

    pub fn run(&self, x: &[f32]) -> Result<Vec<f32>> {
        check("SUM input", x, self.d.iter().product())?;
        let od: [usize; 4] = std::array::from_fn(|i| if self.reduce[i] { 1 } else { self.d[i] });
        let so = bstrides(od);
        let mut out = vec![0f32; self.out_len()];
        let d = self.d;
        if d[3] == 0 {
            return Ok(out);
        }
        for (row, src) in x.chunks_exact(d[3]).enumerate() {
            let (i0, i1, i2) = (row / (d[1] * d[2]), row / d[2] % d[1], row % d[2]);
            let base = i0 * so[0] + i1 * so[1] + i2 * so[2];
            for (i3, v) in src.iter().enumerate() {
                if let Some(o) = out.get_mut(base + i3 * so[3]) {
                    *o += v;
                }
            }
        }
        Ok(out)
    }
}

/// RESIZE_BILINEAR / RESIZE_NEAREST_NEIGHBOR on NHWC.
pub struct Resize {
    pub n: usize,
    pub h: usize,
    pub w: usize,
    pub c: usize,
    pub oh: usize,
    pub ow: usize,
    pub bilinear: bool,
    pub align_corners: bool,
    pub half_pixel: bool,
}

/// Sample positions on one axis: (lower, upper, weight of upper).
fn axis(bilinear: bool, align: bool, half: bool, input: usize, output: usize) -> Vec<(usize, usize, f32)> {
    let scale = if align && output > 1 { input.saturating_sub(1) as f32 / (output - 1) as f32 } else { input as f32 / output.max(1) as f32 };
    let last = input.saturating_sub(1);
    (0..output)
        .map(|o| {
            if bilinear {
                let v = if half { (o as f32 + 0.5) * scale - 0.5 } else { o as f32 * scale };
                let lo = (v.floor().max(0.0) as usize).min(last);
                let hi = (v.ceil().max(0.0) as usize).min(last);
                (lo, hi, v - lo as f32)
            } else {
                let off = if half { 0.5 } else { 0.0 };
                let v = (o as f32 + off) * scale;
                let i = if align { v.round() } else { v.floor() };
                let i = (i.max(0.0) as usize).min(last);
                (i, i, 0.0)
            }
        })
        .collect()
}

impl Resize {
    pub fn out_shape(&self) -> [usize; 4] {
        [self.n, self.oh, self.ow, self.c]
    }

    pub fn run(&self, x: &[f32]) -> Result<Vec<f32>> {
        check("RESIZE input", x, self.n * self.h * self.w * self.c)?;
        let ys = axis(self.bilinear, self.align_corners, self.half_pixel, self.h, self.oh);
        let xs = axis(self.bilinear, self.align_corners, self.half_pixel, self.w, self.ow);
        let c = self.c;
        let mut out = vec![0f32; self.n * self.oh * self.ow * c];
        if c == 0 || self.ow == 0 {
            return Ok(out);
        }
        let px = |b: usize, y: usize, x0: usize| -> &[f32] {
            let at = ((b * self.h + y) * self.w + x0) * c;
            x.get(at..at + c).unwrap_or(&[])
        };
        out.par_chunks_mut(self.ow * c).enumerate().for_each(|(row, dst)| {
            let (b, oy) = (row / self.oh, row % self.oh);
            let Some(&(y0, y1, fy)) = ys.get(oy) else { return };
            for (o, &(x0, x1, fx)) in dst.chunks_exact_mut(c).zip(&xs) {
                if self.bilinear {
                    let (a, bb, cc, d) = (px(b, y0, x0), px(b, y0, x1), px(b, y1, x0), px(b, y1, x1));
                    for (i, o) in o.iter_mut().enumerate() {
                        let g = |s: &[f32]| s.get(i).copied().unwrap_or(0.0);
                        *o = g(a) * (1.0 - fy) * (1.0 - fx) + g(bb) * (1.0 - fy) * fx + g(cc) * fy * (1.0 - fx) + g(d) * fy * fx;
                    }
                } else if let Some(s) = x.get(((b * self.h + y0) * self.w + x0) * c..).and_then(|s| s.get(..c)) {
                    o.copy_from_slice(s);
                }
            }
        });
        Ok(out)
    }
}

/// CONCATENATION along one axis: `outer` blocks, each the inputs' `inner` runs in order.
pub fn concat(xs: &[&[f32]], outer: usize, inners: &[usize], act: Act) -> Result<Vec<f32>> {
    for (x, inner) in xs.iter().zip(inners) {
        check("CONCATENATION input", x, outer * inner)?;
    }
    let mut out = Vec::with_capacity(outer * inners.iter().sum::<usize>());
    for o in 0..outer {
        for (x, &inner) in xs.iter().zip(inners) {
            out.extend_from_slice(x.get(o * inner..(o + 1) * inner).unwrap_or(&[]));
        }
    }
    act.apply_all(&mut out);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plain reference convolution (NHWC, OHWI weights, explicit padding).
    fn conv_ref(x: &[f32], g: &Window, o: usize, w: &[f32], bias: &[f32]) -> Vec<f32> {
        let mut out = vec![0f32; g.n * g.oh * g.ow * o];
        for b in 0..g.n {
            for oy in 0..g.oh {
                for ox in 0..g.ow {
                    for oc in 0..o {
                        let mut acc = bias[oc];
                        for ky in 0..g.kh {
                            for kx in 0..g.kw {
                                let iy = (oy * g.sh + ky * g.dh) as isize - g.pt as isize;
                                let ix = (ox * g.sw + kx * g.dw) as isize - g.pl as isize;
                                if iy < 0 || ix < 0 || iy >= g.h as isize || ix >= g.w as isize {
                                    continue;
                                }
                                for ic in 0..g.c {
                                    acc += x[((b * g.h + iy as usize) * g.w + ix as usize) * g.c + ic] * w[((oc * g.kh + ky) * g.kw + kx) * g.c + ic];
                                }
                            }
                        }
                        out[((b * g.oh + oy) * g.ow + ox) * o + oc] = acc;
                    }
                }
            }
        }
        out
    }

    fn ramp(n: usize, k: f32) -> Vec<f32> {
        (0..n).map(|i| ((i as f32 * k).sin() * 3.0).round() / 2.0).collect()
    }

    fn wmatrix(w: &[f32], o: usize, k: usize) -> Tensor {
        let mut t = vec![0f32; k * o];
        for oc in 0..o {
            for i in 0..k {
                t[i * o + oc] = w[oc * k + i];
            }
        }
        Tensor::from_vec(t, (k, o), &Device::Cpu).unwrap()
    }

    #[test]
    fn same_padding_stride_2() {
        // TFLite: 5 → 3 with k = 3, s = 2: total padding 2, 1 before; 4 → 2: total 1, 0 before
        assert_eq!(pad_before(true, 5, 3, 2, 1), (3, 1));
        assert_eq!(pad_before(true, 4, 3, 2, 1), (2, 0));
        assert_eq!(pad_before(false, 5, 3, 2, 1), (2, 0));
        // dilation 2: effective kernel 5
        assert_eq!(pad_before(true, 6, 3, 1, 2), (6, 2));
        assert_eq!(pad_before(false, 6, 3, 1, 2), (2, 0));
    }

    #[test]
    fn conv_matches_reference() {
        // a 1-channel 4 × 4 image, 3 × 3 kernel of ones, SAME, stride 2: hand-computed sums
        let x: Vec<f32> = (1..=16).map(|v| v as f32).collect();
        let g = Window::new(true, [1, 4, 4, 1], (3, 3), (2, 2), (1, 1));
        assert_eq!((g.oh, g.ow, g.pt, g.pl), (2, 2, 0, 0));
        let conv = Conv { g: g.clone(), o: 1, w: wmatrix(&[1.0; 9], 1, 9), bias: vec![0.5], act: Act::None };
        // windows rows 0-2 × cols 0-2, cols 2-3, rows 2-3 × cols 0-2, rows 2-3 × cols 2-3
        assert_eq!(conv.run(&x).unwrap(), vec![54.5, 45.5, 72.5, 54.5]);

        // random-ish cases against the plain loops, with dilation and several channels
        for (same, hw, k, s, d) in [(true, 7, 3, 2, 1), (false, 9, 3, 1, 2), (true, 8, 2, 2, 1), (true, 6, 1, 1, 1), (true, 5, 5, 2, 1)] {
            let (c, o) = (3, 4);
            let g = Window::new(same, [2, hw, hw, c], (k, k), (s, s), (d, d));
            let x = ramp(2 * hw * hw * c, 0.37);
            let w = ramp(o * k * k * c, 0.91);
            let bias = vec![0.25, -1.0, 0.0, 2.0];
            let conv = Conv { g: g.clone(), o, w: wmatrix(&w, o, k * k * c), bias: bias.clone(), act: Act::None };
            let got = conv.run(&x).unwrap();
            let want = conv_ref(&x, &g, o, &w, &bias);
            for (a, b) in got.iter().zip(&want) {
                assert!((a - b).abs() < 1e-4, "{a} vs {b}");
            }
        }
    }

    #[test]
    fn depthwise_with_multiplier() {
        // 2 channels, multiplier 2: output channel ic·2 + m
        let g = Window::new(false, [1, 3, 3, 2], (2, 2), (1, 1), (1, 1));
        let x: Vec<f32> = (0..18).map(|v| v as f32).collect();
        // weights 2 × 2 × 4: tap (ky, kx) weight for oc = 1 + oc (same per tap)
        let w: Vec<f32> = (0..4).flat_map(|_| [1.0, 2.0, 3.0, 4.0]).collect();
        let dw = Depthwise { g, mult: 2, w, bias: vec![0.0, 0.0, 0.0, 1.0], act: Act::None };
        let out = dw.run(&x).unwrap();
        assert_eq!(dw.out_shape(), [1, 2, 2, 4]);
        // pixel (0, 0): channel 0 window sum = 0 + 2 + 6 + 8 = 16, channel 1 = 1 + 3 + 7 + 9 = 20
        assert_eq!(&out[..4], &[16.0, 32.0, 60.0, 81.0]);
        // dilation 2, SAME: compare with the general convolution of a per-channel kernel
        let g = Window::new(true, [1, 6, 6, 1], (3, 3), (1, 1), (2, 2));
        let x = ramp(36, 0.5);
        let w = ramp(9, 0.3);
        let dw = Depthwise { g: g.clone(), mult: 1, w: w.clone(), bias: vec![0.1], act: Act::Relu6 };
        let want: Vec<f32> = conv_ref(&x, &g, 1, &w, &[0.1]).into_iter().map(|v| v.clamp(0.0, 6.0)).collect();
        assert_eq!(dw.run(&x).unwrap(), want);
    }

    #[test]
    fn transpose_conv_scatters() {
        // 1 × 2 × 2 × 1 input, 2 × 2 kernel, stride 2, no padding: each input pixel paints a block
        let w = [1.0f32, 2.0, 3.0, 4.0]; // o = 1, kh = kw = 2, c = 1
        let tc = TransposeConv {
            n: 1,
            h: 2,
            w: 2,
            c: 1,
            oh: 4,
            ow: 4,
            o: 1,
            kh: 2,
            kw: 2,
            sh: 2,
            sw: 2,
            pt: 0,
            pl: 0,
            weights: Tensor::from_vec(w.to_vec(), (1, 4), &Device::Cpu).unwrap(),
            bias: vec![1.0],
            act: Act::None,
        };
        let out = tc.run(&[1.0, 10.0, 100.0, 1000.0]).unwrap();
        #[rustfmt::skip]
        let want = [
            2.0, 3.0, 11.0, 21.0,
            4.0, 5.0, 31.0, 41.0,
            101.0, 201.0, 1001.0, 2001.0,
            301.0, 401.0, 3001.0, 4001.0,
        ];
        assert_eq!(out, want);
        // overlapping taps (k = 3, stride 2, padding 1 before): output 1 sums two input pixels
        let tc = TransposeConv {
            n: 1,
            h: 1,
            w: 2,
            c: 1,
            oh: 1,
            ow: 4,
            o: 1,
            kh: 1,
            kw: 3,
            sh: 1,
            sw: 2,
            pt: 0,
            pl: 1,
            weights: Tensor::from_vec(vec![1.0f32, 2.0, 3.0], (1, 3), &Device::Cpu).unwrap(),
            bias: vec![0.0],
            act: Act::None,
        };
        // out[ox] = Σ x[i]·w[k] with ox = 2i + k − 1
        assert_eq!(tc.run(&[1.0, 10.0]).unwrap(), vec![2.0, 3.0 + 10.0, 20.0, 30.0]);
    }

    #[test]
    fn resize_modes() {
        // 1 × 2 → 1 × 4 bilinear, half-pixel centers: 0.25 / 0.75 positions, clamped at the ends
        let r = |bilinear, align, half| {
            Resize { n: 1, h: 1, w: 2, c: 1, oh: 1, ow: 4, bilinear, align_corners: align, half_pixel: half }.run(&[0.0, 4.0]).unwrap()
        };
        assert_eq!(r(true, false, true), vec![0.0, 1.0, 3.0, 4.0]);
        // legacy (no half-pixel): positions 0, 0.5, 1, 1.5
        assert_eq!(r(true, false, false), vec![0.0, 2.0, 4.0, 4.0]);
        // align corners: positions 0, 1/3, 2/3, 1
        let a = r(true, true, false);
        assert!((a[1] - 4.0 / 3.0).abs() < 1e-6 && a[3] == 4.0);
        assert_eq!(r(false, false, true), vec![0.0, 0.0, 4.0, 4.0]);
        assert_eq!(r(false, false, false), vec![0.0, 0.0, 4.0, 4.0]);
        // 3 → 2 nearest with half-pixel: (0.5·1.5) = 0.75 → 0, (1.5·1.5) = 2.25 → 2
        let n = Resize { n: 1, h: 1, w: 3, c: 1, oh: 1, ow: 2, bilinear: false, align_corners: false, half_pixel: true };
        assert_eq!(n.run(&[1.0, 2.0, 3.0]).unwrap(), vec![1.0, 3.0]);
    }

    #[test]
    fn elementwise_ops() {
        let b = Binary { op: BinOp::Mul, a: [1, 2, 1, 3], b: [1, 1, 1, 3], out: [1, 2, 1, 3], act: Act::None };
        assert_eq!(b.run(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], &[1.0, 10.0, 100.0]).unwrap(), vec![1.0, 20.0, 300.0, 4.0, 50.0, 600.0]);
        let b = Binary { op: BinOp::Add, a: [1, 2, 1, 1], b: [1, 1, 1, 3], out: [1, 2, 1, 3], act: Act::Relu };
        assert_eq!(b.run(&[1.0, -20.0], &[0.0, 1.0, -2.0]).unwrap(), vec![1.0, 2.0, 0.0, 0.0, 0.0, 0.0]);
        let p = Binary { op: BinOp::Prelu, a: [1, 1, 2, 2], b: [1, 1, 1, 2], out: [1, 1, 2, 2], act: Act::None };
        assert_eq!(p.run(&[-1.0, -1.0, 2.0, -4.0], &[0.5, 0.25]).unwrap(), vec![-0.5, -0.25, 2.0, -1.0]);
        assert!(broadcast([1, 2, 3, 4], [1, 3, 3, 4]).is_err());
        assert!(b.run(&[1.0], &[1.0, 2.0, 3.0]).is_err());

        let s = softmax(&[0.0, 0.0, 1.0, 1.0], 2, 1.0);
        assert_eq!(s, vec![0.5, 0.5, 0.5, 0.5]);
        let l = Unary::Logistic.run(&[0.0]);
        assert_eq!(l, vec![0.5]);
        assert_eq!(Unary::Act(Act::ReluN1To1).run(&[-3.0, 0.5, 3.0]), vec![-1.0, 0.5, 1.0]);

        let sum = Sum { d: [1, 2, 2, 2], reduce: [false, false, true, false] };
        assert_eq!(sum.run(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]).unwrap(), vec![4.0, 6.0, 12.0, 14.0]);

        let t = Transpose { d: [1, 2, 3, 1], perm: [0, 2, 1, 3] };
        assert_eq!(t.out_shape(), [1, 3, 2, 1]);
        assert_eq!(t.run(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]).unwrap(), vec![1.0, 4.0, 2.0, 5.0, 3.0, 6.0]);

        let p = Pad { d: [1, 1, 2, 1], before: [0, 1, 0, 1], out: [1, 2, 2, 3] };
        assert_eq!(p.run(&[7.0, 8.0]).unwrap(), vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 7.0, 0.0, 0.0, 8.0, 0.0]);

        let c = concat(&[&[1.0, 2.0], &[3.0, 4.0, 5.0, 6.0]], 2, &[1, 2], Act::None).unwrap();
        assert_eq!(c, vec![1.0, 3.0, 4.0, 2.0, 5.0, 6.0]);

        let mp = MaxPool { g: Window::new(true, [1, 3, 3, 1], (2, 2), (2, 2), (1, 1)), act: Act::None };
        assert_eq!(mp.run(&[1.0, -2.0, 3.0, -4.0, -5.0, -6.0, 7.0, -8.0, -9.0]).unwrap(), vec![1.0, 3.0, 7.0, -9.0]);
    }
}
