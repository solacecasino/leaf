//! The GPU side: one device, the shader's pipelines compiled once, and
//! buffers kept between pages (made again only when a page needs bigger
//! ones). A page is one submission: upload, every pass, and a copy into a
//! buffer the CPU can read, which goes straight to the output file.

use std::io::Write;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    /// Manga and webtoons: retrace and thin the lines, then sharpen.
    Lines,
    /// Colour comics: the edge-aware upscale and a light sharpen.
    Soft,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    iw: u32,
    ih: u32,
    ow: u32,
    oh: u32,
    rect: [f32; 4],
    strength: [f32; 4],
}

/// How long each part of a page took, in milliseconds.
#[derive(Default, Clone, Copy, Debug)]
pub struct Timing {
    pub upload: f64,
    /// From handing the work over to the result being readable.
    pub gpu: f64,
    /// The shaders alone, as the GPU timed them (when it can).
    pub shaders: Option<f64>,
    pub write: f64,
}

// The buffers the bind group holds, kept alive with it.
#[allow(dead_code)]
struct Buffers {
    src: wgpu::Buffer,
    img: wgpu::Buffer,
    a: wgpu::Buffer,
    b: wgpu::Buffer,
    grad: wgpu::Buffer,
    read: wgpu::Buffer,
    group: wgpu::BindGroup,
    /// What they were made for: source bytes, source pixels, output pixels.
    sizes: (u64, u64, u64),
}

pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pub name: String,
    /// Drawn by the CPU (WARP, llvmpipe): slower than Leaf's own scaler.
    pub software: bool,
    params: wgpu::Buffer,
    layout: wgpu::BindGroupLayout,
    unpack: wgpu::ComputePipeline,
    upscale: wgpu::ComputePipeline,
    gradient: wgpu::ComputePipeline,
    retrace: wgpu::ComputePipeline,
    thin: wgpu::ComputePipeline,
    sharpen: wgpu::ComputePipeline,
    buffers: Option<Buffers>,
    max_buffer: u64,
    /// Timestamps for the passes, when the GPU can take them.
    timer: Option<(wgpu::QuerySet, wgpu::Buffer, wgpu::Buffer, f32)>,
}

impl Gpu {
    /// The best GPU there is; a software one (WARP, lavapipe) if that's all.
    pub fn new() -> Result<Gpu, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .or_else(|_| {
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { force_fallback_adapter: true, ..Default::default() }))
        })
        .map_err(|e| format!("no GPU: {e}"))?;
        let info = adapter.get_info();
        let limits = adapter.limits();
        let timestamps = adapter.features() & wgpu::Features::TIMESTAMP_QUERY;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("upscale-4k"),
            required_limits: limits.clone(),
            required_features: timestamps,
            ..Default::default()
        }))
        .map_err(|e| format!("couldn't use the GPU: {e}"))?;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("upscale"),
            source: wgpu::ShaderSource::Wgsl(include_str!("upscale.wgsl").into()),
        });
        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("upscale"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                storage(1, true),
                storage(2, false),
                storage(3, false),
                storage(4, false),
                storage(5, false),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("upscale"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let max_buffer = (limits.max_storage_buffer_binding_size as u64).min(limits.max_buffer_size);
        let timer = (!timestamps.is_empty()).then(|| {
            let set = device.create_query_set(&wgpu::QuerySetDescriptor { label: None, ty: wgpu::QueryType::Timestamp, count: 2 });
            let resolve = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 16,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let read = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 16,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            (set, resolve, read, queue.get_timestamp_period())
        });
        Ok(Gpu {
            name: format!("{} ({:?})", info.name, info.backend),
            software: info.device_type == wgpu::DeviceType::Cpu,
            unpack: pipeline("unpack_source"),
            upscale: pipeline("upscale"),
            gradient: pipeline("gradient"),
            retrace: pipeline("retrace"),
            thin: pipeline("thin"),
            sharpen: pipeline("sharpen"),
            device,
            queue,
            params,
            layout,
            buffers: None,
            max_buffer,
            timer,
        })
    }

    fn buffers(&mut self, src_bytes: u64, src_px: u64, out_px: u64) -> Result<&Buffers, String> {
        let fits = self.buffers.as_ref().is_some_and(|b| b.sizes.0 >= src_bytes && b.sizes.1 >= src_px && b.sizes.2 >= out_px);
        if !fits {
            // Grown with room to spare, so a run of similar pages reuses them.
            let (sb, sp, op) = self.buffers.as_ref().map_or((0, 0, 0), |b| b.sizes);
            let sizes = (src_bytes.max(sb), src_px.max(sp), out_px.max(op));
            if sizes.0.div_ceil(4) * 4 > self.max_buffer || sizes.1 * 4 > self.max_buffer || sizes.2 * 4 > self.max_buffer {
                return Err("the picture is bigger than this GPU can take in one go".into());
            }
            let make = |label, size: u64, usage| self.device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size.max(4), usage, mapped_at_creation: false });
            let storage = wgpu::BufferUsages::STORAGE;
            let src = make("src", sizes.0.div_ceil(4) * 4, storage | wgpu::BufferUsages::COPY_DST);
            let img = make("img", sizes.1 * 4, storage);
            let a = make("a", sizes.2 * 4, storage);
            let b = make("b", sizes.2 * 4, storage | wgpu::BufferUsages::COPY_SRC);
            let grad = make("grad", sizes.2 * 4, storage);
            let read = make("read", sizes.2 * 4, wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST);
            fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
                wgpu::BindGroupEntry { binding, resource: buffer.as_entire_binding() }
            }
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("upscale"),
                layout: &self.layout,
                entries: &[entry(0, &self.params), entry(1, &src), entry(2, &img), entry(3, &a), entry(4, &b), entry(5, &grad)],
            });
            self.buffers = Some(Buffers { src, img, a, b, grad, read, group, sizes });
        }
        Ok(self.buffers.as_ref().expect("made above"))
    }

    /// Upscales `rgb` (`iw`×`ih`, 3 bytes a pixel) to `ow`×`oh`, covering
    /// `rect` of the source, and writes it to `out` as 0RGB, 4 bytes a
    /// pixel, little-endian.
    #[allow(clippy::too_many_arguments)]
    pub fn run(&mut self, rgb: &[u8], iw: u32, ih: u32, ow: u32, oh: u32, rect: [f32; 4], style: Style, out: &mut impl Write) -> Result<Timing, String> {
        if rgb.len() != iw as usize * ih as usize * 3 || iw == 0 || ih == 0 || ow == 0 || oh == 0 {
            return Err("the picture's size doesn't match its pixels".into());
        }
        let started = Instant::now();
        let (src_px, out_px) = (iw as u64 * ih as u64, ow as u64 * oh as u64);
        let mut strength = match style {
            // retrace, thin, sharpen. Retracing harder than this snaps
            // edges into stair steps on curves (checked by eye at 3×).
            Style::Lines => [0.5, 0.25, 0.35, 0.0],
            Style::Soft => [0.0, 0.0, 0.25, 0.0],
        };
        // For tuning: UPSCALE_STRENGTH="retrace,thin,sharpen".
        if let Ok(v) = std::env::var("UPSCALE_STRENGTH") {
            for (i, x) in v.split(',').filter_map(|x| x.parse().ok()).take(3).enumerate() {
                strength[i] = x;
            }
        }
        self.queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&Params { iw, ih, ow, oh, rect, strength }));
        self.buffers(rgb.len() as u64, src_px, out_px)?;
        let b = self.buffers.as_ref().expect("made above");
        // The upload goes in four-byte steps; the last few bytes are padded.
        let whole = rgb.len() / 4 * 4;
        self.queue.write_buffer(&b.src, 0, &rgb[..whole]);
        if whole < rgb.len() {
            let mut tail = [0u8; 4];
            tail[..rgb.len() - whole].copy_from_slice(&rgb[whole..]);
            self.queue.write_buffer(&b.src, whole as u64, &tail);
        }
        let upload = started.elapsed().as_secs_f64() * 1000.0;

        let mut encoder = self.device.create_command_encoder(&Default::default());
        let groups = |w: u32, h: u32| (w.div_ceil(16), h.div_ceil(16));
        let (sx, sy) = groups(iw, ih);
        let (ox, oy) = groups(ow, oh);
        let lines = [&self.gradient, &self.retrace, &self.thin];
        let refine: &[&wgpu::ComputePipeline] = if style == Style::Lines { &lines } else { &[] };
        // Unpack, upscale, refine (line art), sharpen: one compute pass, timed
        // from its start to its end when the GPU can.
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: None,
            timestamp_writes: self.timer.as_ref().map(|(set, ..)| wgpu::ComputePassTimestampWrites {
                query_set: set,
                beginning_of_pass_write_index: Some(0),
                end_of_pass_write_index: Some(1),
            }),
        });
        pass.set_bind_group(0, &b.group, &[]);
        pass.set_pipeline(&self.unpack);
        pass.dispatch_workgroups(sx, sy, 1);
        for pipeline in std::iter::once(&self.upscale).chain(refine.iter().copied()).chain(std::iter::once(&self.sharpen)) {
            pass.set_pipeline(pipeline);
            pass.dispatch_workgroups(ox, oy, 1);
        }
        drop(pass);
        encoder.copy_buffer_to_buffer(&b.b, 0, &b.read, 0, out_px * 4);
        if let Some((set, resolve, read, _)) = &self.timer {
            encoder.resolve_query_set(set, 0..2, resolve, 0);
            encoder.copy_buffer_to_buffer(resolve, 0, read, 0, 16);
        }
        self.queue.submit([encoder.finish()]);
        let slice = b.read.slice(..out_px * 4);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| drop(tx.send(r)));
        self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).map_err(|e| e.to_string())?;
        rx.recv().map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
        let gpu = started.elapsed().as_secs_f64() * 1000.0 - upload;
        let shaders = self.timer.as_ref().and_then(|(_, _, read, period)| {
            let slice = read.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).ok()?;
            let t: Vec<u64> = slice.get_mapped_range().ok()?.chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap_or_default())).collect();
            read.unmap();
            Some(t[1].saturating_sub(t[0]) as f64 * *period as f64 / 1e6)
        });
        let result = slice.get_mapped_range().map_err(|e| e.to_string()).and_then(|view| out.write_all(&view).map_err(|e| e.to_string()));
        b.read.unmap();
        result?;
        let write = started.elapsed().as_secs_f64() * 1000.0 - upload - gpu;
        Ok(Timing { upload, gpu, shaders, write })
    }
}
