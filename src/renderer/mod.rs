use crate::renderer::shader::ShaderRenderPass;
use anyhow::{Context, Result};
use egui::ClippedMesh;
use egui_wgpu_backend::ScreenDescriptor;
use log::debug;
use wgpu::{
    include_spirv, Adapter, BackendBit, BindGroup, BindGroupDescriptor, BindGroupEntry,
    BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingResource, BindingType, Buffer,
    BufferBinding, BufferBindingType, BufferDescriptor, BufferUsage, Color, ColorTargetState,
    ColorWrite, CommandEncoder, CommandEncoderDescriptor, Device, Extent3d, Features,
    FragmentState, FrontFace, Instance, Limits, LoadOp, MultisampleState, Operations,
    PipelineLayout, PipelineLayoutDescriptor, PolygonMode, PowerPreference, PresentMode,
    PrimitiveState, PrimitiveTopology, PushConstantRange, Queue, RenderBundle,
    RenderBundleDescriptor, RenderBundleEncoderDescriptor, RenderPass, RenderPassColorAttachment,
    RenderPassDescriptor, RenderPipeline, RenderPipelineDescriptor, RequestAdapterOptions,
    ShaderFlags, ShaderModule, ShaderModuleDescriptor, ShaderSource, ShaderStage, Surface,
    SwapChain, SwapChainDescriptor, Texture, TextureDescriptor, TextureFormat, TextureUsage,
    TextureView, TextureViewDescriptor, VertexState,
};
use winit::window::Window;

mod shader;

pub struct GuiData<'a> {
    pub texture: &'a egui::Texture,
    pub paint_jobs: &'a [ClippedMesh],
}

pub struct Renderer {
    #[allow(dead_code)]
    instance: Instance,
    #[allow(dead_code)]
    adapter: Adapter,
    device: Device,

    queue: Queue,
    #[allow(dead_code)]
    surface: Surface,
    format: TextureFormat,
    swapchain: SwapChain,

    vertex_shader: ShaderModule,
    render_tex: Texture,

    shader_rpass: Option<ShaderRenderPass>,
    pub egui_rpass: egui_wgpu_backend::RenderPass,
}

impl Renderer {
    pub async fn new(
        window: &Window,
        power_preference: PowerPreference,
        render_size: (u32, u32),
        push_constants_size: u32,
    ) -> Result<Self> {
        let instance = Instance::new(BackendBit::PRIMARY);
        debug!("Found adapters :");
        instance
            .enumerate_adapters(BackendBit::PRIMARY)
            .for_each(|it| {
                debug!(
                    " - {}: {:?} ({:?})",
                    it.get_info().name,
                    it.get_info().device_type,
                    it.get_info().backend
                );
            });

        // The surface describes where we'll draw our output
        let surface = unsafe { instance.create_surface(window) };

        let adapter = instance
            .request_adapter(&RequestAdapterOptions {
                // Use an integrated gpu if possible
                power_preference,
                compatible_surface: Some(&surface),
            })
            .await
            .context("Can't find a suitable adapter")?;

        debug!(
            "picked : {}: {:?} ({:?})",
            adapter.get_info().name,
            adapter.get_info().device_type,
            adapter.get_info().backend
        );

        // A device is an open connection to a gpu
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("device_request"),
                    features: Features::PUSH_CONSTANTS,
                    limits: Limits {
                        max_push_constant_size: push_constants_size,
                        ..Default::default()
                    },
                },
                None,
            )
            .await?;

        // The output format
        let format = if power_preference == PowerPreference::LowPower {
            TextureFormat::Rgba8UnormSrgb
        } else {
            TextureFormat::Bgra8UnormSrgb
        };
        let window_size = window.inner_size();

        let swapchain = {
            // Here we create the swap chain, which is basically what does the job of
            // rendering our output in sync
            let sc_desc = SwapChainDescriptor {
                usage: TextureUsage::RENDER_ATTACHMENT,
                format,
                width: window_size.width,
                height: window_size.height,
                present_mode: PresentMode::Mailbox,
            };

            device.create_swap_chain(&surface, &sc_desc)
        };

        let render_tex_desc = TextureDescriptor {
            label: Some("yay"),
            size: Extent3d {
                width: render_size.0,
                height: render_size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: TextureUsage::RENDER_ATTACHMENT | TextureUsage::SAMPLED,
        };
        let render_tex = device.create_texture(&render_tex_desc);

        let vertex_shader = device.create_shader_module(&include_spirv!("screen.vert.spv"));

        // The egui renderer in its own render pass
        let mut egui_rpass = egui_wgpu_backend::RenderPass::new(&device, format);
        // egui will need our render texture
        egui_rpass.egui_texture_from_wgpu_texture(&device, &render_tex);

        Ok(Self {
            instance,
            adapter,
            device,
            queue,
            surface,
            format,
            swapchain,
            vertex_shader,
            render_tex,
            // Start with no loaded shader
            shader_rpass: None,
            egui_rpass,
        })
    }

    pub fn set_shader(
        &mut self,
        shader_source: ShaderSource,
        push_constant_size: u32,
        params_buffer_size: u64,
    ) {
        let module = self.device.create_shader_module(&ShaderModuleDescriptor {
            label: Some("nuance fragment shader"),
            source: shader_source,
            flags: ShaderFlags::default(),
        });
        self.shader_rpass = Some(ShaderRenderPass::new(
            &self.device,
            &self.vertex_shader,
            &module,
            push_constant_size,
            params_buffer_size,
            self.format,
        ))
    }

    pub fn render(
        &mut self,
        screen_desc: ScreenDescriptor,
        gui: GuiData,
        params_buffer: &[u8],
        push_constants: &[u8],
    ) -> Result<()> {
        // We use double buffering, so select the output texture
        let frame = self.swapchain.get_current_frame()?.output;
        let view_desc = TextureViewDescriptor::default();

        // This pack a set of render passes for the gpu to execute
        let mut encoder = self
            .device
            .create_command_encoder(&CommandEncoderDescriptor { label: None });

        let render_tex_view = self.render_tex.create_view(&view_desc);

        if let Some(shader_rpass) = self.shader_rpass.as_ref() {
            shader_rpass.update_buffers(&self.queue, params_buffer);
            shader_rpass.execute(&mut encoder, &render_tex_view, push_constants);
        }

        // Upload all resources for the GPU.
        self.egui_rpass
            .update_texture(&self.device, &self.queue, gui.texture);
        self.egui_rpass
            .update_user_textures(&self.device, &self.queue);
        self.egui_rpass
            .update_buffers(&self.device, &self.queue, gui.paint_jobs, &screen_desc);

        // Record all render passes.
        self.egui_rpass.execute(
            &mut encoder,
            &frame.view,
            gui.paint_jobs,
            &screen_desc,
            Some(Color::BLACK),
        );

        // Launch !
        self.queue.submit(Some(encoder.finish()));
        Ok(())
    }
}