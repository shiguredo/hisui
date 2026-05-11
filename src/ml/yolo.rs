use candle_core::{D, DType, Device, IndexOp, Result, Tensor};
use candle_nn::{Conv2dConfig, Module, VarBuilder, batch_norm, conv2d, conv2d_no_bias};
use std::path::Path;

// ============================================================
// YOLOv8 モデル構造 (candle-examples より移植)
// ============================================================

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Multiples {
    depth: f64,
    width: f64,
    ratio: f64,
}

impl Multiples {
    pub fn n() -> Self {
        Self {
            depth: 0.33,
            width: 0.25,
            ratio: 2.0,
        }
    }
    pub fn s() -> Self {
        Self {
            depth: 0.33,
            width: 0.50,
            ratio: 2.0,
        }
    }

    fn filters(&self) -> (usize, usize, usize) {
        let f1 = (256. * self.width) as usize;
        let f2 = (512. * self.width) as usize;
        let f3 = (512. * self.width * self.ratio) as usize;
        (f1, f2, f3)
    }
}

#[derive(Debug)]
struct Upsample {
    scale_factor: usize,
}

impl Upsample {
    fn new(scale_factor: usize) -> Result<Self> {
        Ok(Upsample { scale_factor })
    }
}

impl Module for Upsample {
    fn forward(&self, xs: &Tensor) -> Result<Tensor> {
        let (_b_size, _channels, h, w) = xs.dims4()?;
        xs.upsample_nearest2d(self.scale_factor * h, self.scale_factor * w)
    }
}

#[derive(Debug)]
struct ConvBlock {
    conv: candle_nn::Conv2d,
}

impl ConvBlock {
    fn load(
        vb: VarBuilder,
        c1: usize,
        c2: usize,
        k: usize,
        stride: usize,
        padding: Option<usize>,
    ) -> Result<Self> {
        let padding = padding.unwrap_or(k / 2);
        let cfg = Conv2dConfig {
            padding,
            stride,
            groups: 1,
            dilation: 1,
            cudnn_fwd_algo: None,
        };
        let bn = batch_norm(c2, 1e-3, vb.pp("bn"))?;
        let conv = conv2d_no_bias(c1, c2, k, cfg, vb.pp("conv"))?.absorb_bn(&bn)?;
        Ok(Self { conv })
    }
}

impl Module for ConvBlock {
    fn forward(&self, xs: &Tensor) -> Result<Tensor> {
        let xs = self.conv.forward(xs)?;
        candle_nn::ops::silu(&xs)
    }
}

#[derive(Debug)]
struct Bottleneck {
    cv1: ConvBlock,
    cv2: ConvBlock,
    residual: bool,
}

impl Bottleneck {
    fn load(vb: VarBuilder, c1: usize, c2: usize, shortcut: bool) -> Result<Self> {
        let c_ = c2;
        let cv1 = ConvBlock::load(vb.pp("cv1"), c1, c_, 3, 1, None)?;
        let cv2 = ConvBlock::load(vb.pp("cv2"), c_, c2, 3, 1, None)?;
        let residual = c1 == c2 && shortcut;
        Ok(Self { cv1, cv2, residual })
    }
}

impl Module for Bottleneck {
    fn forward(&self, xs: &Tensor) -> Result<Tensor> {
        let ys = self.cv2.forward(&self.cv1.forward(xs)?)?;
        if self.residual { xs + ys } else { Ok(ys) }
    }
}

#[derive(Debug)]
struct C2f {
    cv1: ConvBlock,
    cv2: ConvBlock,
    bottleneck: Vec<Bottleneck>,
}

impl C2f {
    fn load(vb: VarBuilder, c1: usize, c2: usize, n: usize, shortcut: bool) -> Result<Self> {
        let c = (c2 as f64 * 0.5) as usize;
        let cv1 = ConvBlock::load(vb.pp("cv1"), c1, 2 * c, 1, 1, None)?;
        let cv2 = ConvBlock::load(vb.pp("cv2"), (2 + n) * c, c2, 1, 1, None)?;
        let mut bottleneck = Vec::with_capacity(n);
        for idx in 0..n {
            let b = Bottleneck::load(vb.pp(format!("bottleneck.{idx}")), c, c, shortcut)?;
            bottleneck.push(b)
        }
        Ok(Self {
            cv1,
            cv2,
            bottleneck,
        })
    }
}

impl Module for C2f {
    fn forward(&self, xs: &Tensor) -> Result<Tensor> {
        let ys = self.cv1.forward(xs)?;
        let mut ys = ys.chunk(2, 1)?;
        for m in self.bottleneck.iter() {
            ys.push(m.forward(ys.last().unwrap())?)
        }
        let zs = Tensor::cat(ys.as_slice(), 1)?;
        self.cv2.forward(&zs)
    }
}

#[derive(Debug)]
struct Sppf {
    cv1: ConvBlock,
    cv2: ConvBlock,
    k: usize,
}

impl Sppf {
    fn load(vb: VarBuilder, c1: usize, c2: usize, k: usize) -> Result<Self> {
        let c_ = c1 / 2;
        let cv1 = ConvBlock::load(vb.pp("cv1"), c1, c_, 1, 1, None)?;
        let cv2 = ConvBlock::load(vb.pp("cv2"), c_ * 4, c2, 1, 1, None)?;
        Ok(Self { cv1, cv2, k })
    }
}

impl Module for Sppf {
    fn forward(&self, xs: &Tensor) -> Result<Tensor> {
        let xs = self.cv1.forward(xs)?;
        let xs2 = xs
            .pad_with_zeros(2, self.k / 2, self.k / 2)?
            .pad_with_zeros(3, self.k / 2, self.k / 2)?
            .max_pool2d_with_stride(self.k, 1)?;
        let xs3 = xs2
            .pad_with_zeros(2, self.k / 2, self.k / 2)?
            .pad_with_zeros(3, self.k / 2, self.k / 2)?
            .max_pool2d_with_stride(self.k, 1)?;
        let xs4 = xs3
            .pad_with_zeros(2, self.k / 2, self.k / 2)?
            .pad_with_zeros(3, self.k / 2, self.k / 2)?
            .max_pool2d_with_stride(self.k, 1)?;
        self.cv2.forward(&Tensor::cat(&[&xs, &xs2, &xs3, &xs4], 1)?)
    }
}

#[derive(Debug)]
struct Dfl {
    conv: candle_nn::Conv2d,
    num_classes: usize,
}

impl Dfl {
    fn load(vb: VarBuilder, num_classes: usize) -> Result<Self> {
        let conv = conv2d_no_bias(num_classes, 1, 1, Default::default(), vb.pp("conv"))?;
        Ok(Self { conv, num_classes })
    }
}

impl Module for Dfl {
    fn forward(&self, xs: &Tensor) -> Result<Tensor> {
        let (b_sz, _channels, anchors) = xs.dims3()?;
        let xs = xs
            .reshape((b_sz, 4, self.num_classes, anchors))?
            .transpose(2, 1)?;
        let xs = candle_nn::ops::softmax(&xs, 1)?;
        self.conv.forward(&xs)?.reshape((b_sz, 4, anchors))
    }
}

#[derive(Debug)]
struct DarkNet {
    b1_0: ConvBlock,
    b1_1: ConvBlock,
    b2_0: C2f,
    b2_1: ConvBlock,
    b2_2: C2f,
    b3_0: ConvBlock,
    b3_1: C2f,
    b4_0: ConvBlock,
    b4_1: C2f,
    b5: Sppf,
}

impl DarkNet {
    fn load(vb: VarBuilder, m: Multiples) -> Result<Self> {
        let (w, r, d) = (m.width, m.ratio, m.depth);
        let b1_0 = ConvBlock::load(vb.pp("b1.0"), 3, (64. * w) as usize, 3, 2, Some(1))?;
        let b1_1 = ConvBlock::load(
            vb.pp("b1.1"),
            (64. * w) as usize,
            (128. * w) as usize,
            3,
            2,
            Some(1),
        )?;
        let b2_0 = C2f::load(
            vb.pp("b2.0"),
            (128. * w) as usize,
            (128. * w) as usize,
            (3. * d).round() as usize,
            true,
        )?;
        let b2_1 = ConvBlock::load(
            vb.pp("b2.1"),
            (128. * w) as usize,
            (256. * w) as usize,
            3,
            2,
            Some(1),
        )?;
        let b2_2 = C2f::load(
            vb.pp("b2.2"),
            (256. * w) as usize,
            (256. * w) as usize,
            (6. * d).round() as usize,
            true,
        )?;
        let b3_0 = ConvBlock::load(
            vb.pp("b3.0"),
            (256. * w) as usize,
            (512. * w) as usize,
            3,
            2,
            Some(1),
        )?;
        let b3_1 = C2f::load(
            vb.pp("b3.1"),
            (512. * w) as usize,
            (512. * w) as usize,
            (6. * d).round() as usize,
            true,
        )?;
        let b4_0 = ConvBlock::load(
            vb.pp("b4.0"),
            (512. * w) as usize,
            (512. * w * r) as usize,
            3,
            2,
            Some(1),
        )?;
        let b4_1 = C2f::load(
            vb.pp("b4.1"),
            (512. * w * r) as usize,
            (512. * w * r) as usize,
            (3. * d).round() as usize,
            true,
        )?;
        let b5 = Sppf::load(
            vb.pp("b5.0"),
            (512. * w * r) as usize,
            (512. * w * r) as usize,
            5,
        )?;
        Ok(Self {
            b1_0,
            b1_1,
            b2_0,
            b2_1,
            b2_2,
            b3_0,
            b3_1,
            b4_0,
            b4_1,
            b5,
        })
    }

    fn forward(&self, xs: &Tensor) -> Result<(Tensor, Tensor, Tensor)> {
        let x1 = self.b1_1.forward(&self.b1_0.forward(xs)?)?;
        let x2 = self
            .b2_2
            .forward(&self.b2_1.forward(&self.b2_0.forward(&x1)?)?)?;
        let x3 = self.b3_1.forward(&self.b3_0.forward(&x2)?)?;
        let x4 = self.b4_1.forward(&self.b4_0.forward(&x3)?)?;
        let x5 = self.b5.forward(&x4)?;
        Ok((x2, x3, x5))
    }
}

#[derive(Debug)]
struct YoloV8Neck {
    up: Upsample,
    n1: C2f,
    n2: C2f,
    n3: ConvBlock,
    n4: C2f,
    n5: ConvBlock,
    n6: C2f,
}

impl YoloV8Neck {
    fn load(vb: VarBuilder, m: Multiples) -> Result<Self> {
        let up = Upsample::new(2)?;
        let (w, r, d) = (m.width, m.ratio, m.depth);
        let n = (3. * d).round() as usize;
        let n1 = C2f::load(
            vb.pp("n1"),
            (512. * w * (1. + r)) as usize,
            (512. * w) as usize,
            n,
            false,
        )?;
        let n2 = C2f::load(
            vb.pp("n2"),
            (768. * w) as usize,
            (256. * w) as usize,
            n,
            false,
        )?;
        let n3 = ConvBlock::load(
            vb.pp("n3"),
            (256. * w) as usize,
            (256. * w) as usize,
            3,
            2,
            Some(1),
        )?;
        let n4 = C2f::load(
            vb.pp("n4"),
            (768. * w) as usize,
            (512. * w) as usize,
            n,
            false,
        )?;
        let n5 = ConvBlock::load(
            vb.pp("n5"),
            (512. * w) as usize,
            (512. * w) as usize,
            3,
            2,
            Some(1),
        )?;
        let n6 = C2f::load(
            vb.pp("n6"),
            (512. * w * (1. + r)) as usize,
            (512. * w * r) as usize,
            n,
            false,
        )?;
        Ok(Self {
            up,
            n1,
            n2,
            n3,
            n4,
            n5,
            n6,
        })
    }

    fn forward(&self, p3: &Tensor, p4: &Tensor, p5: &Tensor) -> Result<(Tensor, Tensor, Tensor)> {
        let x = self
            .n1
            .forward(&Tensor::cat(&[&self.up.forward(p5)?, p4], 1)?)?;
        let head_1 = self
            .n2
            .forward(&Tensor::cat(&[&self.up.forward(&x)?, p3], 1)?)?;
        let head_2 = self
            .n4
            .forward(&Tensor::cat(&[&self.n3.forward(&head_1)?, &x], 1)?)?;
        let head_3 = self
            .n6
            .forward(&Tensor::cat(&[&self.n5.forward(&head_2)?, p5], 1)?)?;
        Ok((head_1, head_2, head_3))
    }
}

#[derive(Debug)]
struct DetectionHead {
    dfl: Dfl,
    cv2: [(ConvBlock, ConvBlock, candle_nn::Conv2d); 3],
    cv3: [(ConvBlock, ConvBlock, candle_nn::Conv2d); 3],
    ch: usize,
    no: usize,
}

struct DetectionHeadOut {
    pred: Tensor,
    anchors: Tensor,
    strides: Tensor,
}

fn make_anchors(
    xs0: &Tensor,
    xs1: &Tensor,
    xs2: &Tensor,
    (s0, s1, s2): (usize, usize, usize),
    grid_cell_offset: f64,
) -> Result<(Tensor, Tensor)> {
    let dev = xs0.device();
    let mut anchor_points = vec![];
    let mut stride_tensor = vec![];
    for (xs, stride) in [(xs0, s0), (xs1, s1), (xs2, s2)] {
        let (_, _, h, w) = xs.dims4()?;
        let sx = (Tensor::arange(0, w as u32, dev)?.to_dtype(DType::F32)? + grid_cell_offset)?;
        let sy = (Tensor::arange(0, h as u32, dev)?.to_dtype(DType::F32)? + grid_cell_offset)?;
        let sx = sx
            .reshape((1, sx.elem_count()))?
            .repeat((h, 1))?
            .flatten_all()?;
        let sy = sy
            .reshape((sy.elem_count(), 1))?
            .repeat((1, w))?
            .flatten_all()?;
        anchor_points.push(Tensor::stack(&[&sx, &sy], D::Minus1)?);
        stride_tensor.push((Tensor::ones(h * w, DType::F32, dev)? * stride as f64)?);
    }
    let anchor_points = Tensor::cat(anchor_points.as_slice(), 0)?;
    let stride_tensor = Tensor::cat(stride_tensor.as_slice(), 0)?.unsqueeze(1)?;
    Ok((anchor_points, stride_tensor))
}

fn dist2bbox(distance: &Tensor, anchor_points: &Tensor) -> Result<Tensor> {
    let chunks = distance.chunk(2, 1)?;
    let lt = &chunks[0];
    let rb = &chunks[1];
    Tensor::cat(&[anchor_points.sub(lt)?, anchor_points.add(rb)?], 1)
}

impl DetectionHead {
    fn load(vb: VarBuilder, nc: usize, filters: (usize, usize, usize)) -> Result<Self> {
        let ch = 16;
        let dfl = Dfl::load(vb.pp("dfl"), ch)?;
        let c1 = usize::max(filters.0, nc);
        let c2 = usize::max(filters.0 / 4, ch * 4);
        let cv3 = [
            Self::load_cv3(vb.pp("cv3.0"), c1, nc, filters.0)?,
            Self::load_cv3(vb.pp("cv3.1"), c1, nc, filters.1)?,
            Self::load_cv3(vb.pp("cv3.2"), c1, nc, filters.2)?,
        ];
        let cv2 = [
            Self::load_cv2(vb.pp("cv2.0"), c2, ch, filters.0)?,
            Self::load_cv2(vb.pp("cv2.1"), c2, ch, filters.1)?,
            Self::load_cv2(vb.pp("cv2.2"), c2, ch, filters.2)?,
        ];
        let no = nc + ch * 4;
        Ok(Self {
            dfl,
            cv2,
            cv3,
            ch,
            no,
        })
    }

    fn load_cv3(
        vb: VarBuilder,
        c1: usize,
        nc: usize,
        filter: usize,
    ) -> Result<(ConvBlock, ConvBlock, candle_nn::Conv2d)> {
        let block0 = ConvBlock::load(vb.pp("0"), filter, c1, 3, 1, None)?;
        let block1 = ConvBlock::load(vb.pp("1"), c1, c1, 3, 1, None)?;
        let conv = conv2d(c1, nc, 1, Default::default(), vb.pp("2"))?;
        Ok((block0, block1, conv))
    }

    fn load_cv2(
        vb: VarBuilder,
        c2: usize,
        ch: usize,
        filter: usize,
    ) -> Result<(ConvBlock, ConvBlock, candle_nn::Conv2d)> {
        let block0 = ConvBlock::load(vb.pp("0"), filter, c2, 3, 1, None)?;
        let block1 = ConvBlock::load(vb.pp("1"), c2, c2, 3, 1, None)?;
        let conv = conv2d(c2, 4 * ch, 1, Default::default(), vb.pp("2"))?;
        Ok((block0, block1, conv))
    }

    fn forward(&self, xs0: &Tensor, xs1: &Tensor, xs2: &Tensor) -> Result<DetectionHeadOut> {
        let forward_cv = |xs: &Tensor, i: usize| {
            let xs_2 = self.cv2[i].0.forward(xs)?;
            let xs_2 = self.cv2[i].1.forward(&xs_2)?;
            let xs_2 = self.cv2[i].2.forward(&xs_2)?;
            let xs_3 = self.cv3[i].0.forward(xs)?;
            let xs_3 = self.cv3[i].1.forward(&xs_3)?;
            let xs_3 = self.cv3[i].2.forward(&xs_3)?;
            Tensor::cat(&[&xs_2, &xs_3], 1)
        };
        let xs0 = forward_cv(xs0, 0)?;
        let xs1 = forward_cv(xs1, 1)?;
        let xs2 = forward_cv(xs2, 2)?;

        let (anchors, strides) = make_anchors(&xs0, &xs1, &xs2, (8, 16, 32), 0.5)?;
        let anchors = anchors.transpose(0, 1)?.unsqueeze(0)?;
        let strides = strides.transpose(0, 1)?;

        let reshape = |xs: &Tensor| {
            let d = xs.dim(0)?;
            let el = xs.elem_count();
            xs.reshape((d, self.no, el / (d * self.no)))
        };
        let ys0 = reshape(&xs0)?;
        let ys1 = reshape(&xs1)?;
        let ys2 = reshape(&xs2)?;

        let x_cat = Tensor::cat(&[ys0, ys1, ys2], 2)?;
        let box_ = x_cat.i((.., ..self.ch * 4))?;
        let cls = x_cat.i((.., self.ch * 4..))?;

        let dbox = dist2bbox(&self.dfl.forward(&box_)?, &anchors)?;
        let dbox = dbox.broadcast_mul(&strides)?;
        let pred = Tensor::cat(&[dbox, (cls.neg()?.exp()? + 1.0f64)?.recip()?], 1)?;
        Ok(DetectionHeadOut {
            pred,
            anchors,
            strides,
        })
    }
}

#[derive(Debug)]
pub struct YoloV8 {
    net: DarkNet,
    fpn: YoloV8Neck,
    head: DetectionHead,
}

impl YoloV8 {
    pub fn load(vb: VarBuilder, m: Multiples, num_classes: usize) -> Result<Self> {
        let net = DarkNet::load(vb.pp("net"), m)?;
        let fpn = YoloV8Neck::load(vb.pp("fpn"), m)?;
        let head = DetectionHead::load(vb.pp("head"), num_classes, m.filters())?;
        Ok(Self { net, fpn, head })
    }

    pub fn from_safetensors(path: &Path, multiples: Multiples, device: &Device) -> Result<Self> {
        let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[path], DType::F32, device)? };
        Self::load(vb, multiples, 80)
    }
}

impl Module for YoloV8 {
    fn forward(&self, xs: &Tensor) -> Result<Tensor> {
        let (xs1, xs2, xs3) = self.net.forward(xs)?;
        let (xs1, xs2, xs3) = self.fpn.forward(&xs1, &xs2, &xs3)?;
        Ok(self.head.forward(&xs1, &xs2, &xs3)?.pred)
    }
}

// ============================================================
// YOLOv8-Pose
// ============================================================

#[derive(Debug)]
struct PoseHead {
    detect: DetectionHead,
    cv4: [(ConvBlock, ConvBlock, candle_nn::Conv2d); 3],
    kpt: (usize, usize),
}

impl PoseHead {
    fn load(
        vb: VarBuilder,
        nc: usize,
        kpt: (usize, usize),
        filters: (usize, usize, usize),
    ) -> Result<Self> {
        let detect = DetectionHead::load(vb.clone(), nc, filters)?;
        let nk = kpt.0 * kpt.1;
        let c4 = usize::max(filters.0 / 4, nk);
        let cv4 = [
            Self::load_cv4(vb.pp("cv4.0"), c4, nk, filters.0)?,
            Self::load_cv4(vb.pp("cv4.1"), c4, nk, filters.1)?,
            Self::load_cv4(vb.pp("cv4.2"), c4, nk, filters.2)?,
        ];
        Ok(Self { detect, cv4, kpt })
    }

    fn load_cv4(
        vb: VarBuilder,
        c1: usize,
        nc: usize,
        filter: usize,
    ) -> Result<(ConvBlock, ConvBlock, candle_nn::Conv2d)> {
        let block0 = ConvBlock::load(vb.pp("0"), filter, c1, 3, 1, None)?;
        let block1 = ConvBlock::load(vb.pp("1"), c1, c1, 3, 1, None)?;
        let conv = conv2d(c1, nc, 1, Default::default(), vb.pp("2"))?;
        Ok((block0, block1, conv))
    }

    fn forward(&self, xs0: &Tensor, xs1: &Tensor, xs2: &Tensor) -> Result<Tensor> {
        let d = self.detect.forward(xs0, xs1, xs2)?;
        let forward_cv = |xs: &Tensor, i: usize| {
            let (b_sz, _, h, w) = xs.dims4()?;
            let xs = self.cv4[i].0.forward(xs)?;
            let xs = self.cv4[i].1.forward(&xs)?;
            let xs = self.cv4[i].2.forward(&xs)?;
            xs.reshape((b_sz, self.kpt.0 * self.kpt.1, h * w))
        };
        let xs0 = forward_cv(xs0, 0)?;
        let xs1 = forward_cv(xs1, 1)?;
        let xs2 = forward_cv(xs2, 2)?;
        let xs = Tensor::cat(&[xs0, xs1, xs2], D::Minus1)?;
        let (b_sz, _nk, hw) = xs.dims3()?;
        let xs = xs.reshape((b_sz, self.kpt.0, self.kpt.1, hw))?;

        let ys01 = ((xs.i((.., .., 0..2))? * 2.)?.broadcast_add(&d.anchors)? - 0.5)?
            .broadcast_mul(&d.strides)?;
        let ys2 = (xs.i((.., .., 2..3))?.neg()?.exp()? + 1.0f64)?.recip()?;
        let ys = Tensor::cat(&[ys01, ys2], 2)?.flatten(1, 2)?;
        Tensor::cat(&[d.pred, ys], 1)
    }
}

#[derive(Debug)]
pub struct YoloV8Pose {
    net: DarkNet,
    fpn: YoloV8Neck,
    head: PoseHead,
}

impl YoloV8Pose {
    pub fn load(
        vb: VarBuilder,
        m: Multiples,
        num_classes: usize,
        kpt: (usize, usize),
    ) -> Result<Self> {
        let net = DarkNet::load(vb.pp("net"), m)?;
        let fpn = YoloV8Neck::load(vb.pp("fpn"), m)?;
        let head = PoseHead::load(vb.pp("head"), num_classes, kpt, m.filters())?;
        Ok(Self { net, fpn, head })
    }

    pub fn from_safetensors(path: &Path, multiples: Multiples, device: &Device) -> Result<Self> {
        let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[path], DType::F32, device)? };
        Self::load(vb, multiples, 1, (17, 3))
    }
}

impl Module for YoloV8Pose {
    fn forward(&self, xs: &Tensor) -> Result<Tensor> {
        let (xs1, xs2, xs3) = self.net.forward(xs)?;
        let (xs1, xs2, xs3) = self.fpn.forward(&xs1, &xs2, &xs3)?;
        self.head.forward(&xs1, &xs2, &xs3)
    }
}

// ============================================================
// COCO クラス名
// ============================================================

pub const COCO_CLASSES: [&str; 80] = [
    "person",
    "bicycle",
    "car",
    "motorbike",
    "aeroplane",
    "bus",
    "train",
    "truck",
    "boat",
    "traffic light",
    "fire hydrant",
    "stop sign",
    "parking meter",
    "bench",
    "bird",
    "cat",
    "dog",
    "horse",
    "sheep",
    "cow",
    "elephant",
    "bear",
    "zebra",
    "giraffe",
    "backpack",
    "umbrella",
    "handbag",
    "tie",
    "suitcase",
    "frisbee",
    "skis",
    "snowboard",
    "sports ball",
    "kite",
    "baseball bat",
    "baseball glove",
    "skateboard",
    "surfboard",
    "tennis racket",
    "bottle",
    "wine glass",
    "cup",
    "fork",
    "knife",
    "spoon",
    "bowl",
    "banana",
    "apple",
    "sandwich",
    "orange",
    "broccoli",
    "carrot",
    "hot dog",
    "pizza",
    "donut",
    "cake",
    "chair",
    "sofa",
    "pottedplant",
    "bed",
    "diningtable",
    "toilet",
    "tvmonitor",
    "laptop",
    "mouse",
    "remote",
    "keyboard",
    "cell phone",
    "microwave",
    "oven",
    "toaster",
    "sink",
    "refrigerator",
    "book",
    "clock",
    "vase",
    "scissors",
    "teddy bear",
    "hair drier",
    "toothbrush",
];

// ============================================================
// 検出結果
// ============================================================

#[derive(Debug, Clone)]
pub struct Detection {
    pub class_id: usize,
    pub class_name: &'static str,
    pub confidence: f32,
    pub xmin: f32,
    pub ymin: f32,
    pub xmax: f32,
    pub ymax: f32,
}

#[derive(Debug, Clone)]
pub struct PoseDetection {
    pub detection: Detection,
    pub keypoints: Vec<PoseKeypoint>,
}

#[derive(Debug, Clone, Copy)]
pub struct PoseKeypoint {
    pub x: f32,
    pub y: f32,
    pub confidence: f32,
}

/// COCO 17 点の骨格接続定義 (キーポイント番号のペア)
pub const COCO_SKELETON: [(usize, usize); 19] = [
    (15, 13),
    (13, 11),
    (16, 14),
    (14, 12),
    (11, 12),
    (5, 11),
    (6, 12),
    (5, 6),
    (5, 7),
    (6, 8),
    (7, 9),
    (8, 10),
    (1, 2),
    (0, 1),
    (0, 2),
    (1, 3),
    (2, 4),
    (3, 5),
    (4, 6),
];

// ============================================================
// 検出結果
// ============================================================

// ============================================================
// 前処理: I420 → YOLO 入力テンソル
// ============================================================

/// I420 フレームを YOLOv8 入力テンソルに変換する
///
/// `max_side`: モデル入力の長辺最大サイズ (32 の倍数)
/// 戻り値: (テンソル, (model_w, model_h, orig_w, orig_h))
pub fn preprocess_i420(
    y_plane: &[u8],
    u_plane: &[u8],
    v_plane: &[u8],
    width: usize,
    height: usize,
    max_side: usize,
    device: &Device,
) -> candle_core::Result<(Tensor, (usize, usize, usize, usize))> {
    let uv_width = width.div_ceil(2);

    // モデル入力サイズを計算（長辺 max_side、32 の倍数）
    let max_side_val = if max_side < 32 { 32 } else { max_side };
    let (model_w, model_h) = if width > height {
        let w = max_side_val;
        let h = height * max_side_val / width;
        (w / 32 * 32, h / 32 * 32)
    } else {
        let h = max_side_val;
        let w = width * max_side_val / height;
        (w / 32 * 32, h / 32 * 32)
    };
    let model_uv_w = model_w.div_ceil(2);
    let model_uv_h = model_h.div_ceil(2);

    // libyuv で I420 をモデル入力サイズにスケール
    let mut scaled_y = vec![0u8; model_w * model_h];
    let mut scaled_u = vec![0u8; model_uv_w * model_uv_h];
    let mut scaled_v = vec![0u8; model_uv_w * model_uv_h];

    let src = shiguredo_libyuv::I420Image {
        y: y_plane,
        y_stride: width,
        u: u_plane,
        u_stride: uv_width,
        v: v_plane,
        v_stride: uv_width,
    };
    let mut dst = shiguredo_libyuv::I420ImageMut {
        y: &mut scaled_y,
        y_stride: model_w,
        u: &mut scaled_u,
        u_stride: model_uv_w,
        v: &mut scaled_v,
        v_stride: model_uv_w,
    };
    shiguredo_libyuv::i420_scale(
        &src,
        shiguredo_libyuv::ImageSize::new(width, height),
        &mut dst,
        shiguredo_libyuv::ImageSize::new(model_w, model_h),
        shiguredo_libyuv::FilterMode::Linear,
    )
    .map_err(|e| candle_core::Error::Msg(format!("i420_scale failed: {e}")))?;

    // libyuv で I420 → RGB24 に変換
    let mut rgb = vec![0u8; model_w * model_h * 3];

    let scaled_src = shiguredo_libyuv::I420Image {
        y: &scaled_y,
        y_stride: model_w,
        u: &scaled_u,
        u_stride: model_uv_w,
        v: &scaled_v,
        v_stride: model_uv_w,
    };
    let mut rgb_dst = shiguredo_libyuv::Rgb24ImageMut {
        data: &mut rgb,
        stride: model_w * 3,
    };
    shiguredo_libyuv::i420_to_rgb24(
        &scaled_src,
        &mut rgb_dst,
        shiguredo_libyuv::ImageSize::new(model_w, model_h),
    )
    .map_err(|e| candle_core::Error::Msg(format!("i420_to_rgb24 failed: {e}")))?;

    // Tensor に変換 [1, 3, H, W] f32, [0, 1] 範囲
    let tensor = Tensor::from_vec(rgb, (model_h, model_w, 3), device)?
        .permute((2, 0, 1))? // HWC → CHW
        .unsqueeze(0)? // バッチ次元
        .to_dtype(DType::F32)?
        .affine(1.0 / 255.0, 0.0)?; // 正規化

    Ok((tensor, (model_w, model_h, width, height)))
}

// ============================================================
// 後処理: YOLO 出力 → 検出結果
// ============================================================

/// YOLOv8 の出力から検出結果を抽出する
pub fn postprocess(
    output: &Tensor,
    (model_w, model_h, orig_w, orig_h): (usize, usize, usize, usize),
    confidence_threshold: f32,
    nms_threshold: f32,
) -> candle_core::Result<Vec<Detection>> {
    let pred = output.squeeze(0)?.transpose(0, 1)?; // [84, N] → [N, 84]
    let (num_candidates, _) = pred.dims2()?;
    let pred_data = pred.to_dtype(DType::F32)?.to_vec2::<f32>()?;

    // 各候補に対してクラスごとの検出をグループ化
    let mut bboxes_by_class: Vec<candle_transformers::object_detection::Bbox<(usize, String)>> =
        Vec::new();
    for i in 0..num_candidates {
        let row = &pred_data[i];
        let mut max_conf = 0.0f32;
        let mut class_id = 0usize;
        for c in 0..80 {
            let conf = row[4 + c];
            if conf > max_conf {
                max_conf = conf;
                class_id = c;
            }
        }
        if max_conf < confidence_threshold {
            continue;
        }
        let xmin = row[0];
        let ymin = row[1];
        let xmax = row[2];
        let ymax = row[3];
        bboxes_by_class.push(candle_transformers::object_detection::Bbox {
            xmin,
            ymin,
            xmax,
            ymax,
            confidence: max_conf,
            data: (class_id, COCO_CLASSES[class_id].to_string()),
        });
    }

    // クラスごとにグループ化して NMS
    let mut grouped: Vec<Vec<candle_transformers::object_detection::Bbox<(usize, String)>>> =
        vec![Vec::new(); 80];
    for bbox in bboxes_by_class {
        grouped[bbox.data.0].push(bbox);
    }
    candle_transformers::object_detection::non_maximum_suppression(&mut grouped, nms_threshold);

    // 座標を元画像サイズにスケーリング
    let w_ratio = orig_w as f32 / model_w as f32;
    let h_ratio = orig_h as f32 / model_h as f32;
    let mut detections = Vec::new();
    for class_boxes in &grouped {
        for b in class_boxes {
            detections.push(Detection {
                class_id: b.data.0,
                class_name: COCO_CLASSES[b.data.0],
                confidence: b.confidence,
                xmin: (b.xmin * w_ratio).max(0.0),
                ymin: (b.ymin * h_ratio).max(0.0),
                xmax: (b.xmax * w_ratio).min(orig_w as f32),
                ymax: (b.ymax * h_ratio).min(orig_h as f32),
            });
        }
    }
    Ok(detections)
}

/// YOLOv8-Pose の出力から姿勢推定結果を抽出する
pub fn postprocess_pose(
    output: &Tensor,
    (model_w, model_h, orig_w, orig_h): (usize, usize, usize, usize),
    confidence_threshold: f32,
    nms_threshold: f32,
) -> candle_core::Result<Vec<PoseDetection>> {
    let pred = output.squeeze(0)?.transpose(0, 1)?; // [5+51, N] → [N, 56]
    let (num_candidates, _num_channels) = pred.dims2()?;
    let pred_data = pred.to_dtype(DType::F32)?.to_vec2::<f32>()?;

    let num_classes: usize = 1;
    let kpt_start = 4 + num_classes; // 5

    let mut bboxes_by_class: Vec<
        candle_transformers::object_detection::Bbox<(usize, String, Vec<PoseKeypoint>)>,
    > = Vec::new();
    for i in 0..num_candidates {
        let row = &pred_data[i];
        let mut max_conf = 0.0f32;
        let mut class_id = 0usize;
        for c in 0..num_classes {
            let conf = row[4 + c];
            if conf > max_conf {
                max_conf = conf;
                class_id = c;
            }
        }
        if max_conf < confidence_threshold {
            continue;
        }

        let mut keypoints = Vec::with_capacity(17);
        for k in 0..17 {
            let base = kpt_start + k * 3;
            keypoints.push(PoseKeypoint {
                x: row[base],
                y: row[base + 1],
                confidence: row[base + 2],
            });
        }

        bboxes_by_class.push(candle_transformers::object_detection::Bbox {
            xmin: row[0],
            ymin: row[1],
            xmax: row[2],
            ymax: row[3],
            confidence: max_conf,
            data: (class_id, COCO_CLASSES[class_id].to_string(), keypoints),
        });
    }

    let mut grouped: Vec<
        Vec<candle_transformers::object_detection::Bbox<(usize, String, Vec<PoseKeypoint>)>>,
    > = vec![Vec::new(); num_classes];
    for bbox in bboxes_by_class {
        grouped[bbox.data.0].push(bbox);
    }
    candle_transformers::object_detection::non_maximum_suppression(&mut grouped, nms_threshold);

    let w_ratio = orig_w as f32 / model_w as f32;
    let h_ratio = orig_h as f32 / model_h as f32;
    let mut detections = Vec::new();
    for class_boxes in &grouped {
        for b in class_boxes {
            let mut kpts = b.data.2.clone();
            for kp in &mut kpts {
                kp.x = (kp.x * w_ratio).max(0.0);
                kp.y = (kp.y * h_ratio).max(0.0);
            }
            detections.push(PoseDetection {
                detection: Detection {
                    class_id: b.data.0,
                    class_name: COCO_CLASSES[b.data.0],
                    confidence: b.confidence,
                    xmin: (b.xmin * w_ratio).max(0.0),
                    ymin: (b.ymin * h_ratio).max(0.0),
                    xmax: (b.xmax * w_ratio).min(orig_w as f32),
                    ymax: (b.ymax * h_ratio).min(orig_h as f32),
                },
                keypoints: kpts,
            });
        }
    }
    Ok(detections)
}

// ============================================================
// 描画: 検出結果を I420 フレームに描画
// ============================================================

/// RGB 色
struct Rgb(u8, u8, u8);

/// YUV 色 (BT.601 フルレンジ)
#[derive(Clone, Copy)]
struct YuvColor {
    y: u8,
    u: u8,
    v: u8,
}

/// クラスごとの色パレット
static COLOR_PALETTE: [Rgb; 80] = [
    Rgb(0xFF, 0x1F, 0x1F),
    Rgb(0x1F, 0xFF, 0x1F),
    Rgb(0x1F, 0x1F, 0xFF),
    Rgb(0xFF, 0xFF, 0x1F),
    Rgb(0xFF, 0x1F, 0xFF),
    Rgb(0x1F, 0xFF, 0xFF),
    Rgb(0xFF, 0x7F, 0x1F),
    Rgb(0xFF, 0x1F, 0x7F),
    Rgb(0x7F, 0xFF, 0x1F),
    Rgb(0x1F, 0xFF, 0x7F),
    Rgb(0x7F, 0x1F, 0xFF),
    Rgb(0x1F, 0x7F, 0xFF),
    Rgb(0xFF, 0x7F, 0x7F),
    Rgb(0x7F, 0xFF, 0x7F),
    Rgb(0x7F, 0x7F, 0xFF),
    Rgb(0xFF, 0xFF, 0x7F),
    Rgb(0xFF, 0x7F, 0xFF),
    Rgb(0x7F, 0xFF, 0xFF),
    Rgb(0xFF, 0x3F, 0x1F),
    Rgb(0xFF, 0x1F, 0x3F),
    Rgb(0x3F, 0xFF, 0x1F),
    Rgb(0x1F, 0xFF, 0x3F),
    Rgb(0x3F, 0x1F, 0xFF),
    Rgb(0x1F, 0x3F, 0xFF),
    Rgb(0xFF, 0xBF, 0x1F),
    Rgb(0xFF, 0x1F, 0xBF),
    Rgb(0xBF, 0xFF, 0x1F),
    Rgb(0x1F, 0xFF, 0xBF),
    Rgb(0xBF, 0x1F, 0xFF),
    Rgb(0x1F, 0xBF, 0xFF),
    Rgb(0xBF, 0x7F, 0x1F),
    Rgb(0xBF, 0x1F, 0x7F),
    Rgb(0x7F, 0xBF, 0x1F),
    Rgb(0x1F, 0xBF, 0x7F),
    Rgb(0x7F, 0x1F, 0xBF),
    Rgb(0x1F, 0x7F, 0xBF),
    Rgb(0x7F, 0x3F, 0x1F),
    Rgb(0x7F, 0x1F, 0x3F),
    Rgb(0x3F, 0x7F, 0x1F),
    Rgb(0x1F, 0x7F, 0x3F),
    Rgb(0x3F, 0x1F, 0x7F),
    Rgb(0x1F, 0x3F, 0x7F),
    Rgb(0xFF, 0x9F, 0x1F),
    Rgb(0xFF, 0x1F, 0x9F),
    Rgb(0x9F, 0xFF, 0x1F),
    Rgb(0x1F, 0xFF, 0x9F),
    Rgb(0x9F, 0x1F, 0xFF),
    Rgb(0x1F, 0x9F, 0xFF),
    Rgb(0x9F, 0x5F, 0x1F),
    Rgb(0x9F, 0x1F, 0x5F),
    Rgb(0x5F, 0x9F, 0x1F),
    Rgb(0x1F, 0x9F, 0x5F),
    Rgb(0x5F, 0x1F, 0x9F),
    Rgb(0x1F, 0x5F, 0x9F),
    Rgb(0x5F, 0xFF, 0x1F),
    Rgb(0x5F, 0x1F, 0xFF),
    Rgb(0xFF, 0x5F, 0x1F),
    Rgb(0x1F, 0x5F, 0xFF),
    Rgb(0xFF, 0x1F, 0x5F),
    Rgb(0x1F, 0xFF, 0x5F),
    Rgb(0xDF, 0x5F, 0x1F),
    Rgb(0xDF, 0x1F, 0x5F),
    Rgb(0x5F, 0xDF, 0x1F),
    Rgb(0x1F, 0xDF, 0x5F),
    Rgb(0x5F, 0x1F, 0xDF),
    Rgb(0x1F, 0x5F, 0xDF),
    Rgb(0xDF, 0x9F, 0x1F),
    Rgb(0xDF, 0x1F, 0x9F),
    Rgb(0x9F, 0xDF, 0x1F),
    Rgb(0x1F, 0xDF, 0x9F),
    Rgb(0x9F, 0x1F, 0xDF),
    Rgb(0x1F, 0x9F, 0xDF),
    Rgb(0xBF, 0x3F, 0x7F),
    Rgb(0x3F, 0xBF, 0x7F),
    Rgb(0x7F, 0xBF, 0x3F),
    Rgb(0x7F, 0x3F, 0xBF),
    Rgb(0xCC, 0xCC, 0xFF),
    Rgb(0xFF, 0xCC, 0xCC),
    Rgb(0xCC, 0xFF, 0xCC),
    Rgb(0xFF, 0xFF, 0xCC),
];

fn rgb_to_yuv(r: u8, g: u8, b: u8) -> YuvColor {
    let y = ((77 * r as i32 + 150 * g as i32 + 29 * b as i32 + 128) >> 8).clamp(0, 255) as u8;
    let u =
        (((-43 * r as i32 - 85 * g as i32 + 128 * b as i32 + 128) >> 8) + 128).clamp(0, 255) as u8;
    let v =
        (((128 * r as i32 - 107 * g as i32 - 21 * b as i32 + 128) >> 8) + 128).clamp(0, 255) as u8;
    YuvColor { y, u, v }
}

// 5x7 ビットマップフォント (ASCII 32-126)
static FONT_5X7: [[u8; 5]; 95] = [
    [0x00, 0x00, 0x00, 0x00, 0x00], // ' '
    [0x00, 0x00, 0x5F, 0x00, 0x00], // '!'
    [0x00, 0x07, 0x00, 0x07, 0x00], // '"'
    [0x14, 0x7F, 0x14, 0x7F, 0x14], // '#'
    [0x24, 0x2A, 0x7F, 0x2A, 0x12], // '$'
    [0x23, 0x13, 0x08, 0x64, 0x62], // '%'
    [0x36, 0x49, 0x55, 0x22, 0x50], // '&'
    [0x00, 0x05, 0x03, 0x00, 0x00], // '''
    [0x00, 0x1C, 0x22, 0x41, 0x00], // '('
    [0x00, 0x41, 0x22, 0x1C, 0x00], // ')'
    [0x08, 0x2A, 0x1C, 0x2A, 0x08], // '*'
    [0x08, 0x08, 0x3E, 0x08, 0x08], // '+'
    [0x00, 0x50, 0x30, 0x00, 0x00], // ','
    [0x08, 0x08, 0x08, 0x08, 0x08], // '-'
    [0x00, 0x60, 0x60, 0x00, 0x00], // '.'
    [0x20, 0x10, 0x08, 0x04, 0x02], // '/'
    [0x3E, 0x51, 0x49, 0x45, 0x3E], // '0'
    [0x00, 0x42, 0x7F, 0x40, 0x00], // '1'
    [0x42, 0x61, 0x51, 0x49, 0x46], // '2'
    [0x21, 0x41, 0x45, 0x4B, 0x31], // '3'
    [0x18, 0x14, 0x12, 0x7F, 0x10], // '4'
    [0x27, 0x45, 0x45, 0x45, 0x39], // '5'
    [0x3C, 0x4A, 0x49, 0x49, 0x30], // '6'
    [0x01, 0x71, 0x09, 0x05, 0x03], // '7'
    [0x36, 0x49, 0x49, 0x49, 0x36], // '8'
    [0x06, 0x49, 0x49, 0x29, 0x1E], // '9'
    [0x00, 0x36, 0x36, 0x00, 0x00], // ':'
    [0x00, 0x56, 0x36, 0x00, 0x00], // ';'
    [0x00, 0x08, 0x14, 0x22, 0x41], // '<'
    [0x14, 0x14, 0x14, 0x14, 0x14], // '='
    [0x41, 0x22, 0x14, 0x08, 0x00], // '>'
    [0x02, 0x01, 0x51, 0x09, 0x06], // '?'
    [0x32, 0x49, 0x79, 0x41, 0x3E], // '@'
    [0x7E, 0x11, 0x11, 0x11, 0x7E], // 'A'
    [0x7F, 0x49, 0x49, 0x49, 0x36], // 'B'
    [0x3E, 0x41, 0x41, 0x41, 0x22], // 'C'
    [0x7F, 0x41, 0x41, 0x22, 0x1C], // 'D'
    [0x7F, 0x49, 0x49, 0x49, 0x41], // 'E'
    [0x7F, 0x09, 0x09, 0x01, 0x01], // 'F'
    [0x3E, 0x41, 0x41, 0x51, 0x32], // 'G'
    [0x7F, 0x08, 0x08, 0x08, 0x7F], // 'H'
    [0x00, 0x41, 0x7F, 0x41, 0x00], // 'I'
    [0x20, 0x40, 0x41, 0x3F, 0x01], // 'J'
    [0x7F, 0x08, 0x14, 0x22, 0x41], // 'K'
    [0x7F, 0x40, 0x40, 0x40, 0x40], // 'L'
    [0x7F, 0x02, 0x04, 0x02, 0x7F], // 'M'
    [0x7F, 0x04, 0x08, 0x10, 0x7F], // 'N'
    [0x3E, 0x41, 0x41, 0x41, 0x3E], // 'O'
    [0x7F, 0x09, 0x09, 0x09, 0x06], // 'P'
    [0x3E, 0x41, 0x51, 0x21, 0x5E], // 'Q'
    [0x7F, 0x09, 0x19, 0x29, 0x46], // 'R'
    [0x46, 0x49, 0x49, 0x49, 0x31], // 'S'
    [0x01, 0x01, 0x7F, 0x01, 0x01], // 'T'
    [0x3F, 0x40, 0x40, 0x40, 0x3F], // 'U'
    [0x1F, 0x20, 0x40, 0x20, 0x1F], // 'V'
    [0x7F, 0x20, 0x18, 0x20, 0x7F], // 'W'
    [0x63, 0x14, 0x08, 0x14, 0x63], // 'X'
    [0x03, 0x04, 0x78, 0x04, 0x03], // 'Y'
    [0x61, 0x51, 0x49, 0x45, 0x43], // 'Z'
    [0x00, 0x00, 0x7F, 0x41, 0x41], // '['
    [0x02, 0x04, 0x08, 0x10, 0x20], // '\'
    [0x41, 0x41, 0x7F, 0x00, 0x00], // ']'
    [0x04, 0x02, 0x01, 0x02, 0x04], // '^'
    [0x40, 0x40, 0x40, 0x40, 0x40], // '_'
    [0x00, 0x01, 0x02, 0x04, 0x00], // '`'
    [0x20, 0x54, 0x54, 0x54, 0x78], // 'a'
    [0x7F, 0x48, 0x44, 0x44, 0x38], // 'b'
    [0x38, 0x44, 0x44, 0x44, 0x20], // 'c'
    [0x38, 0x44, 0x44, 0x48, 0x7F], // 'd'
    [0x38, 0x54, 0x54, 0x54, 0x18], // 'e'
    [0x08, 0x7E, 0x09, 0x01, 0x02], // 'f'
    [0x08, 0x14, 0x54, 0x54, 0x3C], // 'g'
    [0x7F, 0x08, 0x04, 0x04, 0x78], // 'h'
    [0x00, 0x44, 0x7D, 0x40, 0x00], // 'i'
    [0x20, 0x40, 0x44, 0x3D, 0x00], // 'j'
    [0x00, 0x7F, 0x10, 0x28, 0x44], // 'k'
    [0x00, 0x41, 0x7F, 0x40, 0x00], // 'l'
    [0x7C, 0x04, 0x18, 0x04, 0x78], // 'm'
    [0x7C, 0x08, 0x04, 0x04, 0x78], // 'n'
    [0x38, 0x44, 0x44, 0x44, 0x38], // 'o'
    [0x7C, 0x14, 0x14, 0x14, 0x08], // 'p'
    [0x08, 0x14, 0x14, 0x18, 0x7C], // 'q'
    [0x7C, 0x08, 0x04, 0x04, 0x08], // 'r'
    [0x48, 0x54, 0x54, 0x54, 0x20], // 's'
    [0x04, 0x3F, 0x44, 0x40, 0x20], // 't'
    [0x3C, 0x40, 0x40, 0x20, 0x7C], // 'u'
    [0x1C, 0x20, 0x40, 0x20, 0x1C], // 'v'
    [0x3C, 0x40, 0x30, 0x40, 0x3C], // 'w'
    [0x44, 0x28, 0x10, 0x28, 0x44], // 'x'
    [0x0C, 0x50, 0x50, 0x50, 0x3C], // 'y'
    [0x44, 0x64, 0x54, 0x4C, 0x44], // 'z'
    [0x00, 0x08, 0x36, 0x41, 0x00], // '{'
    [0x00, 0x00, 0x7F, 0x00, 0x00], // '|'
    [0x00, 0x41, 0x36, 0x08, 0x00], // '}'
    [0x08, 0x04, 0x08, 0x10, 0x08], // '~'
];

/// I420 フレームにバウンディングボックスとラベルを描画する
pub fn draw_detections_on_i420(
    y_plane: &mut [u8],
    u_plane: &mut [u8],
    v_plane: &mut [u8],
    width: usize,
    height: usize,
    detections: &[Detection],
) {
    for det in detections {
        let x1 = det.xmin as usize;
        let y1 = det.ymin as usize;
        let x2 = det.xmax as usize;
        let y2 = det.ymax as usize;
        let color = class_color(det.class_id);
        draw_rect(
            y_plane, u_plane, v_plane, width, height, x1, y1, x2, y2, color,
        );

        // ラベル文字列
        let label = format!("{} {:.0}%", det.class_name, det.confidence * 100.0);
        let text_y = if y1 >= 10 {
            y1.saturating_sub(2)
        } else {
            y2 + 2
        };
        draw_text(
            y_plane, u_plane, v_plane, width, height, x1, text_y, &label, color,
        );
    }
}

fn class_color(class_id: usize) -> YuvColor {
    let rgb = &COLOR_PALETTE[class_id % COLOR_PALETTE.len()];
    rgb_to_yuv(rgb.0, rgb.1, rgb.2)
}

/// I420 フレームに姿勢推定結果を描画する
pub fn draw_pose_on_i420(
    y_plane: &mut [u8],
    u_plane: &mut [u8],
    v_plane: &mut [u8],
    width: usize,
    height: usize,
    detections: &[PoseDetection],
) {
    for det in detections {
        let x1 = det.detection.xmin as usize;
        let y1 = det.detection.ymin as usize;
        let x2 = det.detection.xmax as usize;
        let y2 = det.detection.ymax as usize;
        let color = class_color(det.detection.class_id);

        draw_rect(
            y_plane, u_plane, v_plane, width, height, x1, y1, x2, y2, color,
        );

        let label = format!(
            "{} {:.0}%",
            det.detection.class_name,
            det.detection.confidence * 100.0
        );
        let text_y = if y1 >= 10 {
            y1.saturating_sub(2)
        } else {
            y2 + 2
        };
        draw_text(
            y_plane, u_plane, v_plane, width, height, x1, text_y, &label, color,
        );

        let kp_color = YuvColor { y: 0, u: 0, v: 255 };
        for kp in &det.keypoints {
            if kp.confidence > 0.3 {
                draw_dot(
                    y_plane,
                    u_plane,
                    v_plane,
                    width,
                    height,
                    kp.x as usize,
                    kp.y as usize,
                    2,
                    kp_color,
                );
            }
        }

        let line_color = YuvColor {
            y: 0,
            u: 128,
            v: 255,
        };
        for &(a, b) in &COCO_SKELETON {
            if a < det.keypoints.len() && b < det.keypoints.len() {
                let kp_a = &det.keypoints[a];
                let kp_b = &det.keypoints[b];
                if kp_a.confidence > 0.3 && kp_b.confidence > 0.3 {
                    draw_line(
                        y_plane,
                        u_plane,
                        v_plane,
                        width,
                        height,
                        kp_a.x as usize,
                        kp_a.y as usize,
                        kp_b.x as usize,
                        kp_b.y as usize,
                        line_color,
                    );
                }
            }
        }
    }
}

fn draw_dot(
    y_plane: &mut [u8],
    u_plane: &mut [u8],
    v_plane: &mut [u8],
    frame_w: usize,
    frame_h: usize,
    cx: usize,
    cy: usize,
    radius: usize,
    color: YuvColor,
) {
    let uv_w = frame_w.div_ceil(2);
    for dy in 0..=radius {
        for dx in 0..=radius {
            if dx * dx + dy * dy > radius * radius {
                continue;
            }
            for &(sx, sy) in &[(1isize, 1isize), (-1, 1), (1, -1), (-1, -1)] {
                let px = (cx as isize + dx as isize * sx) as usize;
                let py = (cy as isize + dy as isize * sy) as usize;
                if px < frame_w && py < frame_h {
                    y_plane[py * frame_w + px] = color.y;
                    let ux = px / 2;
                    if ux < uv_w {
                        u_plane[py / 2 * uv_w + ux] = color.u;
                        v_plane[py / 2 * uv_w + ux] = color.v;
                    }
                }
            }
        }
    }
}

fn draw_line(
    y_plane: &mut [u8],
    u_plane: &mut [u8],
    v_plane: &mut [u8],
    frame_w: usize,
    frame_h: usize,
    x1: usize,
    y1: usize,
    x2: usize,
    y2: usize,
    color: YuvColor,
) {
    let uv_w = frame_w.div_ceil(2);
    let dx = (x2 as isize - x1 as isize).abs() as usize;
    let dy = (y2 as isize - y1 as isize).abs() as usize;
    let sx = if x1 < x2 { 1isize } else { -1isize };
    let sy = if y1 < y2 { 1isize } else { -1isize };
    let mut err = dx as isize - dy as isize;
    let mut cx = x1 as isize;
    let mut cy = y1 as isize;

    loop {
        let px = cx as usize;
        let py = cy as usize;
        if px < frame_w && py < frame_h {
            y_plane[py * frame_w + px] = color.y;
            let ux = px / 2;
            if ux < uv_w {
                u_plane[py / 2 * uv_w + ux] = color.u;
                v_plane[py / 2 * uv_w + ux] = color.v;
            }
        }
        if cx == x2 as isize && cy == y2 as isize {
            break;
        }
        let e2 = 2 * err;
        if e2 > -(dy as isize) {
            err -= dy as isize;
            cx += sx;
        }
        if e2 < dx as isize {
            err += dx as isize;
            cy += sy;
        }
    }
}

fn draw_rect(
    y_plane: &mut [u8],
    u_plane: &mut [u8],
    v_plane: &mut [u8],
    frame_w: usize,
    frame_h: usize,
    x1: usize,
    y1: usize,
    x2: usize,
    y2: usize,
    color: YuvColor,
) {
    let x1 = x1.min(frame_w.saturating_sub(1));
    let x2 = x2.min(frame_w.saturating_sub(1));
    let y1 = y1.min(frame_h.saturating_sub(1));
    let y2 = y2.min(frame_h.saturating_sub(1));
    let uv_w = frame_w.div_ceil(2);

    for t in 0..3isize {
        let yt1 = (y1 as isize + t).clamp(0, frame_h as isize - 1) as usize;
        let yt2 = (y2 as isize - t).clamp(0, frame_h as isize - 1) as usize;
        let xt1 = (x1 as isize + t).clamp(0, frame_w as isize - 1) as usize;
        let xt2 = (x2 as isize - t).clamp(0, frame_w as isize - 1) as usize;

        for x in x1.min(xt1)..=x2.max(xt2) {
            if x < frame_w {
                y_plane[yt1 * frame_w + x] = color.y;
                y_plane[yt2 * frame_w + x] = color.y;
                let ux = x / 2;
                if ux < uv_w {
                    u_plane[yt1 / 2 * uv_w + ux] = color.u;
                    u_plane[yt2 / 2 * uv_w + ux] = color.u;
                    v_plane[yt1 / 2 * uv_w + ux] = color.v;
                    v_plane[yt2 / 2 * uv_w + ux] = color.v;
                }
            }
        }
        for y in y1.min(yt1)..=y2.max(yt2) {
            let ux1 = xt1 / 2;
            if ux1 < uv_w {
                u_plane[y / 2 * uv_w + ux1] = color.u;
                v_plane[y / 2 * uv_w + ux1] = color.v;
            }
            let ux2 = xt2 / 2;
            if ux2 < uv_w {
                u_plane[y / 2 * uv_w + ux2] = color.u;
                v_plane[y / 2 * uv_w + ux2] = color.v;
            }
            if xt1 < frame_w {
                y_plane[y * frame_w + xt1] = color.y;
            }
            if xt2 < frame_w {
                y_plane[y * frame_w + xt2] = color.y;
            }
        }
    }
}

fn draw_text(
    y_plane: &mut [u8],
    u_plane: &mut [u8],
    v_plane: &mut [u8],
    frame_w: usize,
    frame_h: usize,
    x: usize,
    y: usize,
    text: &str,
    color: YuvColor,
) {
    let char_w = 6; // 5 + 1 spacing
    let char_h = 8; // 7 + 1 spacing
    let uv_w = frame_w.div_ceil(2);

    // 背景
    let bg = YuvColor {
        y: 0,
        u: 128,
        v: 128,
    };
    for dy in 0..char_h {
        for dx in 0..(text.len() * char_w) {
            let px = x + dx;
            let py = y + dy;
            if px < frame_w && py < frame_h {
                y_plane[py * frame_w + px] = bg.y;
                let ux = px / 2;
                if ux < uv_w {
                    u_plane[py / 2 * uv_w + ux] = bg.u;
                    v_plane[py / 2 * uv_w + ux] = bg.v;
                }
            }
        }
    }

    // 文字描画
    for (ci, ch) in text.chars().enumerate() {
        let glyph_idx = if (ch as u32) >= 32 && (ch as u32) < 127 {
            (ch as u32 - 32) as usize
        } else {
            0
        };
        let glyph = &FONT_5X7[glyph_idx];
        for col in 0..5 {
            let bits = glyph[col];
            for row in 0..7 {
                if (bits >> row) & 1 != 0 {
                    let px = x + ci * char_w + col;
                    let py = y + row + 1; // +1 for top margin
                    if px < frame_w && py < frame_h {
                        y_plane[py * frame_w + px] = color.y;
                        let ux = px / 2;
                        if ux < uv_w {
                            u_plane[py / 2 * uv_w + ux] = color.u;
                            v_plane[py / 2 * uv_w + ux] = color.v;
                        }
                    }
                }
            }
        }
    }
}
