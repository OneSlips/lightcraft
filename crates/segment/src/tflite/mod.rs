//! A small, bounded TensorFlow Lite interpreter in pure Rust, for the official MediaPipe models
//! LightCraft runs unchanged: the multiclass selfie segmenter (`selfie_multiclass_256x256`), the
//! BlazeFace short-range face detector and the FaceMesh-V2 landmark model (`face_detector` and
//! `face_landmarks_detector` of `face_landmarker.task`), Apache-2.0, Copyright Google LLC.
//!
//! It reads the model's FlatBuffer ([`fb`], bounds-checked), dequantizes the `float16` weights
//! once at load, checks every operator's shapes, pre-arranges the convolution weights for matrix
//! products, and runs the graph in order on `f32` NHWC buffers. Only what these models use is
//! supported: tensor types `FLOAT32`, `FLOAT16` (constants, through `DEQUANTIZE`) and `INT32`
//! (constant shapes and axes), operators `CONV_2D`, `DEPTHWISE_CONV_2D`, `TRANSPOSE_CONV`, `ADD`,
//! `MUL`, `PRELU`, `RELU`, `RELU6`, `RELU_N1_TO_1`, `LOGISTIC`, `PAD`, `MAX_POOL_2D`, `RESHAPE`,
//! `TRANSPOSE`, `SOFTMAX`, `SUM`, `CONCATENATION`, `RESIZE_BILINEAR`, `RESIZE_NEAREST_NEIGHBOR` and
//! `DEQUANTIZE`, tensors of rank ≤ 4. Anything else is a clear error at load, never a panic.
//!
//! Written by the LightCraft contributors in 2026 from the TensorFlow Lite model schema
//! (`tensorflow/compiler/mlir/lite/schema/schema.fbs`: table field order, enum values and option
//! defaults; Apache-2.0, Copyright The TensorFlow Authors) and the documented behaviour of its
//! builtin operators; no TensorFlow code was copied. [`zip`] reads the stored entries of a
//! MediaPipe `.task` bundle.

mod fb;
mod ops;
pub mod zip;

use std::path::Path;

use candle_core::{Device, Tensor};

use crate::{Error, Result};
use fb::Table;
use ops::{Act, BinOp, Binary, Conv, Depthwise, MaxPool, Pad, Resize, Sum, Transpose, TransposeConv, Unary, Window};

/// Largest tensor (elements): 256 MB of `f32`.
const MAX_ELEMS: usize = 64 << 20;
/// Most operators in a graph.
const MAX_OPS: usize = 8192;
/// Most tensors in a graph.
const MAX_TENSORS: usize = 1 << 16;
/// Largest model file.
const MAX_FILE: u64 = 1 << 30;

fn bad(msg: &str) -> Error {
    Error::Message(format!("TFLite model: {msg}"))
}

// TensorType
const FLOAT32: u8 = 0;
const FLOAT16: u8 = 1;
const INT32: u8 = 2;

// BuiltinOperator
const ADD: i32 = 0;
const CONCATENATION: i32 = 2;
const CONV_2D: i32 = 3;
const DEPTHWISE_CONV_2D: i32 = 4;
const DEQUANTIZE: i32 = 6;
const LOGISTIC: i32 = 14;
const MAX_POOL_2D: i32 = 17;
const MUL: i32 = 18;
const RELU: i32 = 19;
const RELU_N1_TO_1: i32 = 20;
const RELU6: i32 = 21;
const RESHAPE: i32 = 22;
const RESIZE_BILINEAR: i32 = 23;
const SOFTMAX: i32 = 25;
const PAD: i32 = 34;
const TRANSPOSE: i32 = 39;
const PRELU: i32 = 54;
const TRANSPOSE_CONV: i32 = 67;
const SUM: i32 = 74;
const RESIZE_NEAREST_NEIGHBOR: i32 = 97;

fn op_name(code: i32) -> String {
    let n = match code {
        ADD => "ADD",
        CONCATENATION => "CONCATENATION",
        CONV_2D => "CONV_2D",
        DEPTHWISE_CONV_2D => "DEPTHWISE_CONV_2D",
        DEQUANTIZE => "DEQUANTIZE",
        LOGISTIC => "LOGISTIC",
        MAX_POOL_2D => "MAX_POOL_2D",
        MUL => "MUL",
        RELU => "RELU",
        RELU_N1_TO_1 => "RELU_N1_TO_1",
        RELU6 => "RELU6",
        RESHAPE => "RESHAPE",
        RESIZE_BILINEAR => "RESIZE_BILINEAR",
        SOFTMAX => "SOFTMAX",
        PAD => "PAD",
        TRANSPOSE => "TRANSPOSE",
        PRELU => "PRELU",
        TRANSPOSE_CONV => "TRANSPOSE_CONV",
        SUM => "SUM",
        RESIZE_NEAREST_NEIGHBOR => "RESIZE_NEAREST_NEIGHBOR",
        32 => "CUSTOM",
        c => return format!("builtin operator {c}"),
    };
    n.to_string()
}

/// `float16` bits to `f32` (exact).
fn f16_to_f32(h: u16) -> f32 {
    let sign = ((h & 0x8000) as u32) << 16;
    let exp = ((h >> 10) & 0x1f) as u32;
    let man = (h & 0x3ff) as u32;
    let bits = match exp {
        0 => {
            // zero or subnormal: man · 2^-24
            let v = man as f32 * (1.0 / 16_777_216.0);
            return if sign != 0 { -v } else { v };
        }
        31 => sign | 0x7f80_0000 | (man << 13),
        e => sign | ((e + 112) << 23) | (man << 13),
    };
    f32::from_bits(bits)
}

/// A tensor of the graph as declared in the file.
struct TensorDef {
    shape: Vec<usize>,
    dtype: u8,
    name: String,
}

/// A constant tensor's values.
enum Const {
    F(Vec<f32>),
    I(Vec<i32>),
}

/// What an operator step computes.
enum Kind {
    Conv(Conv),
    Depthwise(Depthwise),
    TransposeConv(TransposeConv),
    MaxPool(MaxPool),
    Binary(Binary),
    Unary(Unary),
    Pad(Pad),
    Transpose(Transpose),
    Softmax { inner: usize, beta: f32 },
    Sum(Sum),
    Resize(Resize),
    Concat { outer: usize, inners: Vec<usize>, act: Act },
    /// RESHAPE (and DEQUANTIZE of an `f32` tensor): the same values.
    Copy,
}

struct Step {
    kind: Kind,
    /// The tensors the kernel reads (data operands only, in kernel order).
    ins: Vec<usize>,
    out: usize,
}

/// A loaded TFLite model, ready to run (thread-safe; [`Model::run`] takes `&self`).
pub struct Model {
    tensors: Vec<TensorDef>,
    consts: Vec<Option<Vec<f32>>>,
    steps: Vec<Step>,
    inputs: Vec<usize>,
    outputs: Vec<usize>,
    /// Tensors no longer needed after each step.
    free_after: Vec<Vec<usize>>,
}

/// Shape as 4-D (leading ones).
fn dims4(s: &[usize]) -> Result<[usize; 4]> {
    if s.len() > 4 {
        return Err(bad(&format!("tensors of rank {} are not supported (≤ 4)", s.len())));
    }
    let mut d = [1; 4];
    for (i, v) in s.iter().enumerate() {
        d[4 - s.len() + i] = *v;
    }
    Ok(d)
}

fn numel(s: &[usize]) -> Result<usize> {
    s.iter()
        .try_fold(1usize, |a, &d| a.checked_mul(d))
        .filter(|&n| n <= MAX_ELEMS)
        .ok_or_else(|| bad(&format!("tensor of shape {s:?} is too large")))
}

/// Normalize a possibly negative axis of a rank-`rank` tensor.
fn axis(a: i32, rank: usize) -> Result<usize> {
    let r = rank as i64;
    let v = if a < 0 { a as i64 + r } else { a as i64 };
    if v < 0 || v >= r.max(1) {
        return Err(bad(&format!("axis {a} out of range for rank {rank}")));
    }
    Ok(v as usize)
}

/// The parse/compile state.
struct Builder {
    tensors: Vec<TensorDef>,
    consts: Vec<Option<Const>>,
    /// Whether a tensor has a value at run time (graph input or produced by an earlier step).
    live: Vec<bool>,
}

impl Builder {
    fn def(&self, t: i32) -> Result<&TensorDef> {
        usize::try_from(t).ok().and_then(|t| self.tensors.get(t)).ok_or_else(|| bad(&format!("tensor index {t} out of range")))
    }

    fn shape(&self, t: i32) -> Result<Vec<usize>> {
        Ok(self.def(t)?.shape.clone())
    }

    fn floats(&self, t: i32, what: &str) -> Result<&[f32]> {
        match usize::try_from(t).ok().and_then(|t| self.consts.get(t)) {
            Some(Some(Const::F(v))) => Ok(v),
            _ => Err(bad(&format!("{what} must be a constant float tensor"))),
        }
    }

    fn ints(&self, t: i32, what: &str) -> Result<&[i32]> {
        match usize::try_from(t).ok().and_then(|t| self.consts.get(t)) {
            Some(Some(Const::I(v))) => Ok(v),
            _ => Err(bad(&format!("{what} must be a constant int32 tensor"))),
        }
    }

    /// A data operand: a live value or a float constant.
    fn data(&self, t: i32) -> Result<usize> {
        let i = usize::try_from(t).map_err(|_| bad("missing operand"))?;
        let ok = self.live.get(i).copied().unwrap_or(false) || matches!(self.consts.get(i), Some(Some(Const::F(_))));
        if !ok {
            return Err(bad(&format!("operand {i} ({}) is used before it is computed", self.def(t)?.name)));
        }
        Ok(i)
    }
}

fn input(ins: &[i32], i: usize) -> i32 {
    ins.get(i).copied().unwrap_or(-1)
}

fn usize_of(v: i32, what: &str) -> Result<usize> {
    usize::try_from(v).ok().filter(|&v| v > 0 && v <= 1 << 16).ok_or_else(|| bad(&format!("{what} {v} is not valid")))
}

/// Read `Model`'s graph from FlatBuffer bytes.
fn parse(buf: &[u8]) -> Result<Model> {
    let model = Table::root(buf)?;
    // Model: version 0, operator_codes 1, subgraphs 2, description 3, buffers 4
    let codes_v = model.vector(1, 4)?;
    let mut codes = Vec::with_capacity(codes_v.len().min(MAX_OPS));
    for i in 0..codes_v.len().min(MAX_OPS) {
        // OperatorCode: deprecated_builtin_code 0 (byte), custom_code 1, version 2, builtin_code 3 (int)
        let c = codes_v.table(i)?;
        codes.push((c.i8(0, 0)? as i32).max(c.i32(3, 0)?));
    }
    let subgraphs = model.vector(2, 4)?;
    if subgraphs.is_empty() {
        return Err(bad("no subgraph"));
    }
    let g = subgraphs.table(0)?;
    let buffers = model.vector(4, 4)?;

    // SubGraph: tensors 0, inputs 1, outputs 2, operators 3
    let tv = g.vector(0, 4)?;
    if tv.len() > MAX_TENSORS {
        return Err(bad(&format!("{} tensors (at most {MAX_TENSORS})", tv.len())));
    }
    let mut b = Builder { tensors: Vec::with_capacity(tv.len()), consts: Vec::with_capacity(tv.len()), live: vec![false; tv.len()] };
    for i in 0..tv.len() {
        // Tensor: shape 0, type 1, buffer 2, name 3
        let t = tv.table(i)?;
        let shape: Vec<usize> = t
            .vector(0, 4)?
            .i32s()?
            .into_iter()
            .map(|d| usize::try_from(d).map_err(|_| bad(&format!("tensor {i} has a dynamic or negative dimension {d}"))))
            .collect::<Result<_>>()?;
        if shape.len() > 8 {
            return Err(bad(&format!("tensor {i} has rank {}", shape.len())));
        }
        let n = numel(&shape)?;
        let dtype = t.u8(1, 0)?;
        let name = t.string(3)?;
        let bi = t.u32(2, 0)? as usize;
        let data: &[u8] = if bi == 0 || bi >= buffers.len() {
            &[]
        } else {
            // Buffer: data 0, offset 1, size 2 (offset/size: data stored after the FlatBuffer)
            let bt = buffers.table(bi)?;
            let d = bt.vector(0, 1)?.bytes();
            let (off, size) = (bt.u64(1, 0)?, bt.u64(2, 0)?);
            if d.is_empty() && off > 1 {
                let (off, size) = (usize::try_from(off).ok(), usize::try_from(size).ok());
                off.zip(size).and_then(|(o, s)| buf.get(o..o.checked_add(s)?)).ok_or_else(|| bad(&format!("tensor {name}: data outside the file")))?
            } else {
                d
            }
        };
        let c = if data.is_empty() {
            None
        } else {
            let want = |w: usize| -> Result<()> {
                if data.len() != n * w {
                    return Err(bad(&format!("tensor {name}: {} bytes of data for shape {shape:?}", data.len())));
                }
                Ok(())
            };
            Some(match dtype {
                FLOAT32 => {
                    want(4)?;
                    Const::F(data.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
                }
                FLOAT16 => {
                    want(2)?;
                    Const::F(data.chunks_exact(2).map(|c| f16_to_f32(u16::from_le_bytes([c[0], c[1]]))).collect())
                }
                INT32 => {
                    want(4)?;
                    Const::I(data.chunks_exact(4).map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
                }
                t => return Err(bad(&format!("tensor {name}: type {t} is not supported (FLOAT32, FLOAT16, INT32 are)"))),
            })
        };
        b.tensors.push(TensorDef { shape, dtype, name });
        b.consts.push(c);
    }

    let index = |v: i32| -> Result<usize> {
        usize::try_from(v).ok().filter(|&v| v < tv.len()).ok_or_else(|| bad(&format!("tensor index {v} out of range")))
    };
    let inputs: Vec<usize> = g.vector(1, 4)?.i32s()?.into_iter().map(index).collect::<Result<_>>()?;
    let outputs: Vec<usize> = g.vector(2, 4)?.i32s()?.into_iter().map(index).collect::<Result<_>>()?;
    for &i in &inputs {
        let t = &b.tensors[i];
        if t.dtype != FLOAT32 {
            return Err(bad(&format!("input {} is not float32", t.name)));
        }
        b.live[i] = true;
    }

    let ov = g.vector(3, 4)?;
    if ov.len() > MAX_OPS {
        return Err(bad(&format!("{} operators (at most {MAX_OPS})", ov.len())));
    }
    let mut steps = Vec::with_capacity(ov.len());
    for i in 0..ov.len() {
        // Operator: opcode_index 0, inputs 1, outputs 2, builtin_options_type 3, builtin_options 4
        let op = ov.table(i)?;
        let code = codes.get(op.u32(0, 0)? as usize).copied().ok_or_else(|| bad("operator code index out of range"))?;
        let ins = op.vector(1, 4)?.i32s()?;
        let outs = op.vector(2, 4)?.i32s()?;
        let opts = op.table(4)?;
        let [out] = outs[..] else {
            return Err(bad(&format!("operator {i} ({}) has {} outputs", op_name(code), outs.len())));
        };
        let out = index(out)?;
        if b.tensors[out].dtype != FLOAT32 {
            return Err(bad(&format!("operator {i} ({}): output {} is not float32", op_name(code), b.tensors[out].name)));
        }
        if b.consts[out].is_some() || b.live[out] {
            return Err(bad(&format!("operator {i} ({}): output {} is written twice", op_name(code), b.tensors[out].name)));
        }
        let step = compile(&mut b, code, &ins, out, opts).map_err(|e| match e {
            Error::Message(m) => Error::Message(format!("{m} (operator {i}, {})", op_name(code))),
            e => e,
        })?;
        if let Some(s) = step {
            steps.push(s);
            b.live[out] = true;
        }
    }
    for &o in &outputs {
        if !b.live[o] && !matches!(b.consts[o], Some(Const::F(_))) {
            return Err(bad(&format!("output {} is never computed", b.tensors[o].name)));
        }
    }

    // free each value after its last reader (graph outputs are kept)
    let mut last = vec![None; b.tensors.len()];
    for (si, s) in steps.iter().enumerate() {
        for &t in &s.ins {
            last[t] = Some(si);
        }
    }
    let mut free_after = vec![Vec::new(); steps.len()];
    for (t, l) in last.iter().enumerate() {
        if let Some(l) = l
            && !outputs.contains(&t)
            && b.consts[t].is_none()
        {
            free_after[*l].push(t);
        }
    }
    let consts = b.consts.into_iter().map(|c| if let Some(Const::F(v)) = c { Some(v) } else { None }).collect();
    Ok(Model { tensors: b.tensors, consts, steps, inputs, outputs, free_after })
}

/// Check the computed output shape against the declared one.
fn expect_shape(b: &Builder, out: usize, got: &[usize]) -> Result<()> {
    let want = &b.tensors[out].shape;
    if want.as_slice() != got {
        return Err(bad(&format!("output {} has shape {want:?}, the operator computes {got:?}", b.tensors[out].name)));
    }
    Ok(())
}

fn bias_of(b: &Builder, t: i32, n: usize) -> Result<Vec<f32>> {
    if t < 0 {
        return Ok(vec![0.0; n]);
    }
    let v = b.floats(t, "bias")?;
    if v.len() != n {
        return Err(bad(&format!("bias has {} values for {n} channels", v.len())));
    }
    Ok(v.to_vec())
}

fn opt_table<'a>(opts: Option<Table<'a>>, what: &str) -> Result<Table<'a>> {
    opts.ok_or_else(|| bad(&format!("{what} options missing")))
}

/// Padding enum: SAME = 0, VALID = 1.
fn same(p: i8) -> Result<bool> {
    match p {
        0 => Ok(true),
        1 => Ok(false),
        p => Err(bad(&format!("padding {p}"))),
    }
}

fn compile(b: &mut Builder, code: i32, ins: &[i32], out: usize, opts: Option<Table>) -> Result<Option<Step>> {
    let step = |kind: Kind, ins: Vec<usize>| Ok(Some(Step { kind, ins, out }));
    match code {
        DEQUANTIZE | RESHAPE => {
            let x = input(ins, 0);
            let n_in = numel(&b.shape(x)?)?;
            if numel(&b.tensors[out].shape)? != n_in {
                return Err(bad(&format!("{} values reshaped to {:?}", n_in, b.tensors[out].shape)));
            }
            // constants fold (the float16 weights become float32 here, once)
            if let Ok(v) = b.floats(x, "x") {
                let v = v.to_vec();
                b.consts[out] = Some(Const::F(v));
                return Ok(None);
            }
            if b.def(x)?.dtype != FLOAT32 {
                return Err(bad("only constant float16 tensors can be dequantized"));
            }
            step(Kind::Copy, vec![b.data(x)?])
        }
        CONV_2D => {
            let o = opt_table(opts, "CONV_2D")?;
            // Conv2DOptions: padding 0, stride_w 1, stride_h 2, fused_activation 3, dilation_w 4, dilation_h 5
            let (x, wt) = (input(ins, 0), input(ins, 1));
            let d = dims4(&b.shape(x)?)?;
            let ws = b.shape(wt)?;
            let [oc, kh, kw, ic] = ws[..] else { return Err(bad("weights must be [out, h, w, in]")) };
            if ic != d[3] {
                return Err(bad(&format!("weights for {ic} input channels, the input has {} (grouped convolutions are not supported)", d[3])));
            }
            let g = Window::new(
                same(o.i8(0, 0)?)?,
                d,
                (kh, kw),
                (usize_of(o.i32(2, 0)?, "stride")?, usize_of(o.i32(1, 0)?, "stride")?),
                (usize_of(o.i32(5, 1)?, "dilation")?, usize_of(o.i32(4, 1)?, "dilation")?),
            );
            let w = b.floats(wt, "CONV_2D weights")?;
            let k = kh * kw * ic;
            // [o, k] → [k, o]
            let mut t = vec![0f32; w.len()];
            for (oi, row) in w.chunks_exact(k.max(1)).enumerate() {
                for (ki, v) in row.iter().enumerate() {
                    if let Some(d) = t.get_mut(ki * oc + oi) {
                        *d = *v;
                    }
                }
            }
            let conv = Conv {
                g,
                o: oc,
                w: Tensor::from_vec(t, (k, oc), &Device::Cpu)?,
                bias: bias_of(b, input(ins, 2), oc)?,
                act: Act::from_code(o.i8(3, 0)?)?,
            };
            expect_shape(b, out, &conv.out_shape())?;
            step(Kind::Conv(conv), vec![b.data(x)?])
        }
        DEPTHWISE_CONV_2D => {
            let o = opt_table(opts, "DEPTHWISE_CONV_2D")?;
            // DepthwiseConv2DOptions: padding 0, stride_w 1, stride_h 2, depth_multiplier 3,
            // fused_activation 4, dilation_w 5, dilation_h 6
            let (x, wt) = (input(ins, 0), input(ins, 1));
            let d = dims4(&b.shape(x)?)?;
            let ws = b.shape(wt)?;
            let [1, kh, kw, oc] = ws[..] else { return Err(bad("weights must be [1, h, w, channels]")) };
            if d[3] == 0 || !oc.is_multiple_of(d[3]) {
                return Err(bad(&format!("{oc} output channels for {} input channels", d[3])));
            }
            let dw = Depthwise {
                g: Window::new(
                    same(o.i8(0, 0)?)?,
                    d,
                    (kh, kw),
                    (usize_of(o.i32(2, 0)?, "stride")?, usize_of(o.i32(1, 0)?, "stride")?),
                    (usize_of(o.i32(6, 1)?, "dilation")?, usize_of(o.i32(5, 1)?, "dilation")?),
                ),
                mult: oc / d[3],
                w: b.floats(wt, "DEPTHWISE_CONV_2D weights")?.to_vec(),
                bias: bias_of(b, input(ins, 2), oc)?,
                act: Act::from_code(o.i8(4, 0)?)?,
            };
            expect_shape(b, out, &dw.out_shape())?;
            step(Kind::Depthwise(dw), vec![b.data(x)?])
        }
        TRANSPOSE_CONV => {
            let o = opt_table(opts, "TRANSPOSE_CONV")?;
            // TransposeConvOptions: padding 0, stride_w 1, stride_h 2, fused_activation 3
            // inputs: output_shape, weights [o, h, w, in], input, bias (optional)
            let (shape_t, wt, x) = (input(ins, 0), input(ins, 1), input(ins, 2));
            let os: Vec<usize> = b.ints(shape_t, "TRANSPOSE_CONV output shape")?.iter().map(|&v| usize::try_from(v).unwrap_or(0)).collect();
            let [n, oh, ow, oc] = os[..] else { return Err(bad("output shape must have 4 values")) };
            let d = dims4(&b.shape(x)?)?;
            let ws = b.shape(wt)?;
            let [woc, kh, kw, ic] = ws[..] else { return Err(bad("weights must be [out, h, w, in]")) };
            if woc != oc || ic != d[3] || n != d[0] {
                return Err(bad(&format!("weights {ws:?} don't fit input {d:?} and output {os:?}")));
            }
            let (sh, sw) = (usize_of(o.i32(2, 0)?, "stride")?, usize_of(o.i32(1, 0)?, "stride")?);
            let same = same(o.i8(0, 0)?)?;
            // the padding of the forward convolution from the output back to the input
            let (_, pt) = ops::pad_before(same, oh, kh, sh, 1);
            let (_, pl) = ops::pad_before(same, ow, kw, sw, 1);
            let w = b.floats(wt, "TRANSPOSE_CONV weights")?;
            // [o, kh, kw, in] → [in, (kh, kw, o)]
            let kko = kh * kw * oc;
            let mut t = vec![0f32; w.len()];
            for (i, v) in w.iter().enumerate() {
                let (oi, rest) = (i / (kh * kw * ic).max(1), i % (kh * kw * ic).max(1));
                let (tap, ci) = (rest / ic.max(1), rest % ic.max(1));
                if let Some(d) = t.get_mut(ci * kko + tap * oc + oi) {
                    *d = *v;
                }
            }
            let tc = TransposeConv {
                n,
                h: d[1],
                w: d[2],
                c: ic,
                oh,
                ow,
                o: oc,
                kh,
                kw,
                sh,
                sw,
                pt,
                pl,
                weights: Tensor::from_vec(t, (ic, kko), &Device::Cpu)?,
                bias: bias_of(b, input(ins, 3), oc)?,
                act: Act::from_code(o.i8(3, 0)?)?,
            };
            numel(&os)?;
            expect_shape(b, out, &tc.out_shape())?;
            step(Kind::TransposeConv(tc), vec![b.data(x)?])
        }
        MAX_POOL_2D => {
            let o = opt_table(opts, "MAX_POOL_2D")?;
            // Pool2DOptions: padding 0, stride_w 1, stride_h 2, filter_w 3, filter_h 4, fused_activation 5
            let x = input(ins, 0);
            let d = dims4(&b.shape(x)?)?;
            let mp = MaxPool {
                g: Window::new(
                    same(o.i8(0, 0)?)?,
                    d,
                    (usize_of(o.i32(4, 0)?, "filter")?, usize_of(o.i32(3, 0)?, "filter")?),
                    (usize_of(o.i32(2, 0)?, "stride")?, usize_of(o.i32(1, 0)?, "stride")?),
                    (1, 1),
                ),
                act: Act::from_code(o.i8(5, 0)?)?,
            };
            expect_shape(b, out, &mp.out_shape())?;
            step(Kind::MaxPool(mp), vec![b.data(x)?])
        }
        ADD | MUL | PRELU => {
            // AddOptions / MulOptions: fused_activation 0
            let act = match opts {
                Some(o) if code != PRELU => Act::from_code(o.i8(0, 0)?)?,
                _ => Act::None,
            };
            let (x, y) = (input(ins, 0), input(ins, 1));
            let (sx, sy) = (b.shape(x)?, b.shape(y)?);
            let (a, c) = (dims4(&sx)?, dims4(&sy)?);
            let shape = ops::broadcast(a, c)?;
            let rank = sx.len().max(sy.len());
            expect_shape(b, out, shape.get(4 - rank..).unwrap_or(&[]))?;
            let op = match code {
                ADD => BinOp::Add,
                MUL => BinOp::Mul,
                _ => BinOp::Prelu,
            };
            step(Kind::Binary(Binary { op, a, b: c, out: shape, act }), vec![b.data(x)?, b.data(y)?])
        }
        RELU | RELU6 | RELU_N1_TO_1 | LOGISTIC => {
            let x = input(ins, 0);
            expect_shape(b, out, &b.shape(x)?)?;
            let u = match code {
                RELU => Unary::Act(Act::Relu),
                RELU6 => Unary::Act(Act::Relu6),
                RELU_N1_TO_1 => Unary::Act(Act::ReluN1To1),
                _ => Unary::Logistic,
            };
            step(Kind::Unary(u), vec![b.data(x)?])
        }
        PAD => {
            let (x, p) = (input(ins, 0), input(ins, 1));
            let s = b.shape(x)?;
            let pads = b.ints(p, "PAD paddings")?;
            if pads.len() != 2 * s.len() || pads.iter().any(|&v| !(0..=1 << 16).contains(&v)) {
                return Err(bad(&format!("paddings {pads:?} for rank {}", s.len())));
            }
            let lead = 4usize.saturating_sub(s.len());
            let d = dims4(&s)?;
            let mut before = [0; 4];
            let mut outd = d;
            for i in 0..s.len() {
                let (lo, hi) = (pads.get(2 * i).copied().unwrap_or(0) as usize, pads.get(2 * i + 1).copied().unwrap_or(0) as usize);
                before[lead + i] = lo;
                outd[lead + i] += lo + hi;
            }
            numel(&outd)?;
            expect_shape(b, out, outd.get(lead..).unwrap_or(&[]))?;
            step(Kind::Pad(Pad { d, before, out: outd }), vec![b.data(x)?])
        }
        TRANSPOSE => {
            let (x, p) = (input(ins, 0), input(ins, 1));
            let s = b.shape(x)?;
            let perm = b.ints(p, "TRANSPOSE permutation")?;
            let lead = 4usize.saturating_sub(s.len());
            let mut p4 = [0, 1, 2, 3];
            let mut seen = [false; 4];
            if perm.len() != s.len() {
                return Err(bad(&format!("permutation {perm:?} for rank {}", s.len())));
            }
            for (i, &v) in perm.iter().enumerate() {
                let a = axis(v, s.len())? + lead;
                if std::mem::replace(&mut seen[a], true) {
                    return Err(bad(&format!("permutation {perm:?} repeats an axis")));
                }
                p4[lead + i] = a;
            }
            let t = Transpose { d: dims4(&s)?, perm: p4 };
            expect_shape(b, out, t.out_shape().get(lead..).unwrap_or(&[]))?;
            step(Kind::Transpose(t), vec![b.data(x)?])
        }
        SOFTMAX => {
            // SoftmaxOptions: beta 0
            let beta = opt_table(opts, "SOFTMAX")?.f32(0, 0.0)?;
            let x = input(ins, 0);
            let s = b.shape(x)?;
            expect_shape(b, out, &s)?;
            step(Kind::Softmax { inner: s.last().copied().unwrap_or(1), beta }, vec![b.data(x)?])
        }
        SUM => {
            // ReducerOptions: keep_dims 0
            let keep = opts.map(|o| o.bool(0)).transpose()?.unwrap_or(false);
            let (x, a) = (input(ins, 0), input(ins, 1));
            let s = b.shape(x)?;
            let lead = 4usize.saturating_sub(s.len());
            let mut reduce = [false; 4];
            for &v in b.ints(a, "SUM axes")? {
                reduce[axis(v, s.len())? + lead] = true;
            }
            let shape: Vec<usize> = s
                .iter()
                .enumerate()
                .filter_map(|(i, &d)| match (reduce[lead + i], keep) {
                    (true, true) => Some(1),
                    (true, false) => None,
                    (false, _) => Some(d),
                })
                .collect();
            expect_shape(b, out, &shape)?;
            step(Kind::Sum(Sum { d: dims4(&s)?, reduce }), vec![b.data(x)?])
        }
        RESIZE_BILINEAR | RESIZE_NEAREST_NEIGHBOR => {
            // ResizeBilinearOptions: new_height 0, new_width 1 (deprecated), align_corners 2,
            // half_pixel_centers 3; ResizeNearestNeighborOptions: align_corners 0, half_pixel_centers 1
            let bilinear = code == RESIZE_BILINEAR;
            let (ac, hp) = if bilinear { (2, 3) } else { (0, 1) };
            let (align_corners, half_pixel) = match opts {
                Some(o) => (o.bool(ac)?, o.bool(hp)?),
                None => (false, false),
            };
            let (x, sz) = (input(ins, 0), input(ins, 1));
            let s = b.ints(sz, "resize size")?;
            let [oh, ow] = s[..] else { return Err(bad("resize size must have 2 values")) };
            let d = b.shape(x)?;
            let [n, h, w, c] = d[..] else { return Err(bad("resize input must be 4-D")) };
            let r = Resize { n, h, w, c, oh: usize_of(oh, "size")?, ow: usize_of(ow, "size")?, bilinear, align_corners, half_pixel };
            numel(&r.out_shape())?;
            expect_shape(b, out, &r.out_shape())?;
            step(Kind::Resize(r), vec![b.data(x)?])
        }
        CONCATENATION => {
            let o = opt_table(opts, "CONCATENATION")?;
            // ConcatenationOptions: axis 0, fused_activation 1
            let first = b.shape(input(ins, 0))?;
            let ax = axis(o.i32(0, 0)?, first.len())?;
            let outer: usize = first.get(..ax).unwrap_or(&[]).iter().product();
            let mut shape = first.clone();
            shape[ax] = 0;
            let mut inners = Vec::new();
            let mut data = Vec::new();
            for &t in ins {
                let s = b.shape(t)?;
                if s.len() != first.len() || s.iter().zip(&first).enumerate().any(|(i, (a, f))| i != ax && a != f) {
                    return Err(bad(&format!("can't concatenate {s:?} with {first:?} on axis {ax}")));
                }
                shape[ax] += s[ax];
                inners.push(s.get(ax..).unwrap_or(&[]).iter().product());
                data.push(b.data(t)?);
            }
            expect_shape(b, out, &shape)?;
            step(Kind::Concat { outer, inners, act: Act::from_code(o.i8(1, 0)?)? }, data)
        }
        c => Err(bad(&format!("operator {} is not supported", op_name(c)))),
    }
}

impl Model {
    /// Parse and validate a `.tflite` FlatBuffer, dequantize its constants and prepare its
    /// weights. Errors (never panics) on a malformed file or an unsupported operator.
    pub fn from_bytes(bytes: &[u8]) -> Result<Model> {
        parse(bytes)
    }

    /// [`Model::from_bytes`] on a file.
    pub fn load(path: &Path) -> Result<Model> {
        let len = std::fs::metadata(path).map_err(|e| Error::Message(format!("can't read {}: {e}", path.display())))?.len();
        if len > MAX_FILE {
            return Err(bad(&format!("{} is too large ({len} bytes)", path.display())));
        }
        let bytes = std::fs::read(path).map_err(|e| Error::Message(format!("can't read {}: {e}", path.display())))?;
        Self::from_bytes(&bytes).map_err(|e| Error::Message(format!("{}: {e}", path.display())))
    }

    /// The shapes of the graph inputs (NHWC for images).
    pub fn input_shapes(&self) -> Vec<Vec<usize>> {
        self.inputs.iter().map(|&i| self.tensors.get(i).map(|t| t.shape.clone()).unwrap_or_default()).collect()
    }

    /// The shapes of the graph outputs, in model order.
    pub fn output_shapes(&self) -> Vec<Vec<usize>> {
        self.outputs.iter().map(|&i| self.tensors.get(i).map(|t| t.shape.clone()).unwrap_or_default()).collect()
    }

    /// Run the graph: one row-major `f32` buffer per input (in model order, NHWC), the outputs in
    /// model order.
    pub fn run(&self, inputs: &[&[f32]]) -> Result<Vec<Vec<f32>>> {
        let vals = self.execute(inputs, false)?;
        self.outputs
            .iter()
            .map(|&o| match (vals.get(o).and_then(|v| v.as_ref()), self.consts.get(o).and_then(|c| c.as_ref())) {
                (Some(v), _) | (None, Some(v)) => Ok(v.clone()),
                _ => Err(bad("an output was not computed")),
            })
            .collect()
    }

    /// Every computed tensor by name (for comparisons with a reference interpreter).
    #[doc(hidden)]
    pub fn run_traced(&self, inputs: &[&[f32]]) -> Result<Vec<(String, Vec<f32>)>> {
        let vals = self.execute(inputs, true)?;
        Ok(vals.into_iter().zip(&self.tensors).filter_map(|(v, t)| Some((t.name.clone(), v?))).collect())
    }

    fn execute(&self, inputs: &[&[f32]], keep: bool) -> Result<Vec<Option<Vec<f32>>>> {
        if inputs.len() != self.inputs.len() {
            return Err(bad(&format!("{} inputs given, the model takes {}", inputs.len(), self.inputs.len())));
        }
        let mut vals: Vec<Option<Vec<f32>>> = vec![None; self.tensors.len()];
        for (&t, x) in self.inputs.iter().zip(inputs) {
            let def = self.tensors.get(t).ok_or_else(|| bad("input index"))?;
            let n = numel(&def.shape)?;
            if x.len() != n {
                return Err(bad(&format!("input {} takes {n} values ({:?}), {} given", def.name, def.shape, x.len())));
            }
            if let Some(v) = vals.get_mut(t) {
                *v = Some(x.to_vec());
            }
        }
        for (si, s) in self.steps.iter().enumerate() {
            let get = |t: usize| -> Result<&[f32]> {
                vals.get(t)
                    .and_then(|v| v.as_deref())
                    .or_else(|| self.consts.get(t).and_then(|c| c.as_deref()))
                    .ok_or_else(|| bad(&format!("tensor {t} has no value")))
            };
            let x = get(*s.ins.first().ok_or_else(|| bad("operator without input"))?)?;
            let y = match &s.kind {
                Kind::Conv(k) => k.run(x)?,
                Kind::Depthwise(k) => k.run(x)?,
                Kind::TransposeConv(k) => k.run(x)?,
                Kind::MaxPool(k) => k.run(x)?,
                Kind::Binary(k) => k.run(x, get(*s.ins.get(1).ok_or_else(|| bad("missing operand"))?)?)?,
                Kind::Unary(k) => k.run(x),
                Kind::Pad(k) => k.run(x)?,
                Kind::Transpose(k) => k.run(x)?,
                Kind::Softmax { inner, beta } => ops::softmax(x, *inner, *beta),
                Kind::Sum(k) => k.run(x)?,
                Kind::Resize(k) => k.run(x)?,
                Kind::Concat { outer, inners, act } => {
                    let xs = s.ins.iter().map(|&t| get(t)).collect::<Result<Vec<_>>>()?;
                    ops::concat(&xs, *outer, inners, *act)?
                }
                Kind::Copy => x.to_vec(),
            };
            if let Some(v) = vals.get_mut(s.out) {
                *v = Some(y);
            }
            if !keep {
                for &t in self.free_after.get(si).map(|v| v.as_slice()).unwrap_or(&[]) {
                    if let Some(v) = vals.get_mut(t) {
                        *v = None;
                    }
                }
            }
        }
        Ok(vals)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_floats_convert_exactly() {
        assert_eq!(f16_to_f32(0x3c00), 1.0);
        assert_eq!(f16_to_f32(0xc000), -2.0);
        assert_eq!(f16_to_f32(0x3555), 0.333_251_95);
        assert_eq!(f16_to_f32(0x7bff), 65504.0);
        assert_eq!(f16_to_f32(0x0001), 5.960_464_5e-8);
        assert_eq!(f16_to_f32(0x8000), -0.0);
        assert!(f16_to_f32(0x7c00).is_infinite());
        assert!(f16_to_f32(0x7e00).is_nan());
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(Model::from_bytes(&[]).is_err());
        assert!(Model::from_bytes(&[0; 64]).is_err());
        let noise: Vec<u8> = (0..4096u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
        assert!(Model::from_bytes(&noise).is_err());
        assert!(Model::load(Path::new("/nonexistent/model.tflite")).is_err());
    }
}
