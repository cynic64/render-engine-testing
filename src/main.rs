// Copyright (c) 2016 The vulkano developers
// Licensed under the Apache License, Version 2.0
// <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT
// license <LICENSE-MIT or http://opensource.org/licenses/MIT>,
// at your option. All files in the project carrying such
// notice may not be copied, modified, or distributed except
// according to those terms.

// Welcome to the triangle example!
//
// This is the only example that is entirely detailed. All the other examples avoid code
// duplication by using helper functions.
//
// This example assumes that you are already more or less familiar with graphics programming
// and that you want to learn Vulkan. This means that for example it won't go into details about
// what a vertex or a shader is.

use vulkano::buffer::{BufferUsage, CpuAccessibleBuffer};
use vulkano::command_buffer::{AutoCommandBuffer, AutoCommandBufferBuilder, DynamicState};
use vulkano::descriptor::PipelineLayoutAbstract;
use vulkano::device::{Device, DeviceExtensions, Queue};
use vulkano::framebuffer::{Framebuffer, FramebufferAbstract, RenderPassAbstract, Subpass};
use vulkano::image::SwapchainImage;
use vulkano::instance::{Instance, PhysicalDevice};
use vulkano::pipeline::vertex::SingleBufferDefinition;
use vulkano::pipeline::viewport::Viewport;
use vulkano::pipeline::GraphicsPipeline;
use vulkano::swapchain;
use vulkano::swapchain::{
    AcquireError, PresentMode, Surface, SurfaceTransform, Swapchain, SwapchainCreationError,
};
use vulkano::sync;
use vulkano::sync::{FlushError, GpuFuture};

use vulkano_win::VkSurfaceBuild;

use winit::{Event, EventsLoop, Window, WindowBuilder, WindowEvent};

use std::sync::Arc;

#[derive(Debug, Clone)]
struct Vertex {
    position: [f32; 2],
}
vulkano::impl_vertex!(Vertex, position);

type VertexBuffer = CpuAccessibleBuffer<[Vertex]>;

type ConcreteGraphicsPipeline = GraphicsPipeline<
    SingleBufferDefinition<Vertex>,
    Box<PipelineLayoutAbstract + Send + Sync + 'static>,
    Arc<RenderPassAbstract + Send + Sync + 'static>,
>;

struct App {
    instance: Arc<Instance>,
    events_loop: EventsLoop,
    surface: Arc<Surface<Window>>,
    physical_device_index: usize,
    device: Arc<Device>,
    queue: Arc<Queue>,
    // present_queue: Arc<Queue>,
    swapchain: Arc<Swapchain<Window>>,
    images: Vec<Arc<SwapchainImage<Window>>>,
    // swapchain_images: Vec<Arc<SwapchainImage<Window>>>,
    render_pass: Arc<RenderPassAbstract + Send + Sync>,
    pipeline: Arc<ConcreteGraphicsPipeline>,
    dynamic_state: DynamicState,
    framebuffers: Vec<Arc<FramebufferAbstract + Send + Sync>>,
    recreate_swapchain: bool,
    previous_frame_end: Box<GpuFuture>,
    done: bool,
    start_time: std::time::Instant,
    // swapchain_framebuffers: Vec<Arc<FramebufferAbstract + Send + Sync>>,
    frame_data: FrameData,
}

struct FrameData {
    image_num: Option<usize>,
    acquire_future: Option<vulkano::swapchain::SwapchainAcquireFuture<Window>>,
    vertex_buffers: Vec<Arc<VertexBuffer>>,
    command_buffer: Option<AutoCommandBuffer>,
}

fn main() {
    let mut app = App::new();
    app.run();
}

impl App {
    fn new() -> Self {
        let instance = get_instance();
        let physical = get_physical_device(&instance);
        println!(
            "Using device: {} (type: {:?}, index: {})",
            physical.name(),
            physical.ty(),
            physical.index()
        );

        // The objective of this example is to draw a triangle on a window. To do so, we first need to
        // create the window.
        //
        // This is done by creating a `WindowBuilder` from the `winit` crate, then calling the
        // `build_vk_surface` method provided by the `VkSurfaceBuild` trait from `vulkano_win`. If you
        // ever get an error about `build_vk_surface` being undefined in one of your projects, this
        // probably means that you forgot to import this trait.
        //
        // This returns a `vulkano::swapchain::Surface` object that contains both a cross-platform winit
        // window and a cross-platform Vulkan surface that represents the surface of the window.
        let events_loop = EventsLoop::new();
        let surface = WindowBuilder::new()
            .build_vk_surface(&events_loop, instance.clone())
            .unwrap();

        let (device, mut queues) = get_device_and_queues(physical, surface.clone());

        // Since we can request multiple queues, the `queues` variable is in fact an iterator. In this
        // example we use only one queue, so we just retrieve the first and only element of the
        // iterator and throw it away.
        let queue = queues.next().unwrap();

        let (swapchain, images) = create_swapchain_and_images(
            physical.clone(),
            surface.clone(),
            device.clone(),
            queue.clone(),
        );

        // The next step is to create the shaders.
        //
        // The raw shader creation API provided by the vulkano library is unsafe, for various reasons.
        //
        // An overview of what the `vulkano_shaders::shader!` macro generates can be found in the
        // `vulkano-shaders` crate docs. You can view them at https://docs.rs/vulkano-shaders/
        //
        // TODO: explain this in details
        mod vs {
            vulkano_shaders::shader! {
                ty: "vertex",
                src: "
    #version 450

    layout(location = 0) in vec2 position;

    void main() {
        gl_Position = vec4(position, 0.0, 1.0);
    }"
            }
        }

        mod fs {
            vulkano_shaders::shader! {
                ty: "fragment",
                src: "
    #version 450

    layout(location = 0) out vec4 f_color;

    void main() {
        f_color = vec4(1.0, 0.0, 0.0, 1.0);
    }
    "
            }
        }

        let vs = vs::Shader::load(device.clone()).unwrap();
        let fs = fs::Shader::load(device.clone()).unwrap();

        // At this point, OpenGL initialization would be finished. However in Vulkan it is not. OpenGL
        // implicitly does a lot of computation whenever you draw. In Vulkan, you have to do all this
        // manually.

        // The next step is to create a *render pass*, which is an object that describes where the
        // output of the graphics pipeline will go. It describes the layout of the images
        // where the colors, depth and/or stencil information will be written.
        let render_pass: Arc<RenderPassAbstract + Send + Sync> = Arc::new(
            vulkano::single_pass_renderpass!(
                device.clone(),
                attachments: {
                    // `color` is a custom name we give to the first and only attachment.
                    color: {
                        // `load: Clear` means that we ask the GPU to clear the content of this
                        // attachment at the start of the drawing.
                        load: Clear,
                        // `store: Store` means that we ask the GPU to store the output of the draw
                        // in the actual image. We could also ask it to discard the result.
                        store: Store,
                        // `format: <ty>` indicates the type of the format of the image. This has to
                        // be one of the types of the `vulkano::format` module (or alternatively one
                        // of your structs that implements the `FormatDesc` trait). Here we use the
                        // same format as the swapchain.
                        format: swapchain.format(),
                        // TODO:
                        samples: 1,
                    }
                },
                pass: {
                    // We use the attachment named `color` as the one and only color attachment.
                    color: [color],
                    // No depth-stencil attachment is indicated with empty brackets.
                    depth_stencil: {}
                }
            )
            .unwrap(),
        );

        // Before we draw we have to create what is called a pipeline. This is similar to an OpenGL
        // program, but much more specific.
        let pipeline = Arc::new(
            GraphicsPipeline::start()
                // We need to indicate the layout of the vertices.
                // The type `SingleBufferDefinition` actually contains a template parameter corresponding
                // to the type of each vertex. But in this code it is automatically inferred.
                .vertex_input_single_buffer()
                // A Vulkan shader can in theory contain multiple entry points, so we have to specify
                // which one. The `main` word of `main_entry_point` actually corresponds to the name of
                // the entry point.
                .vertex_shader(vs.main_entry_point(), ())
                // The content of the vertex buffer describes a list of triangles.
                .triangle_list()
                // Use a resizable viewport set to draw over the entire window
                .viewports_dynamic_scissors_irrelevant(1)
                // See `vertex_shader`.
                .fragment_shader(fs.main_entry_point(), ())
                // We have to indicate which subpass of which render pass this pipeline is going to be used
                // in. The pipeline will only be usable from this particular subpass.
                .render_pass(Subpass::from(render_pass.clone(), 0).unwrap())
                // Now that our builder is filled, we call `build()` to obtain an actual pipeline.
                .build(device.clone())
                .unwrap(),
        );

        // Dynamic viewports allow us to recreate just the viewport when the window is resized
        // Otherwise we would have to recreate the whole pipeline.
        let mut dynamic_state = DynamicState {
            line_width: None,
            viewports: None,
            scissors: None,
        };

        // The render pass we created above only describes the layout of our framebuffers. Before we
        // can draw we also need to create the actual framebuffers.
        //
        // Since we need to draw to multiple images, we are going to create a different framebuffer for
        // each image.
        let framebuffers =
            window_size_dependent_setup(&images, render_pass.clone(), &mut dynamic_state);

        // Initialization is finally finished!

        // In some situations, the swapchain will become invalid by it This includes for example
        // when the window is resized (as the images of the swapchain will no longer match the
        // window's) or, on Android, when the application went to the background and goes back to the
        // foreground.
        //
        // In this situation, acquiring a swapchain image or presenting it will return an error.
        // Rendering to an image of that swapchain will not produce any error, but may or may not work.
        // To continue rendering, we need to recreate the swapchain by creating a new swapchain.
        // Here, we remember that we need to do this for the next loop iteration.
        let recreate_swapchain = false;

        // In the loop below we are going to submit commands to the GPU. Submitting a command produces
        // an object that implements the `GpuFuture` trait, which holds the resources for as long as
        // they are in use by the GPU.
        //
        // Destroying the `GpuFuture` blocks until the GPU is finished executing it. In order to avoid
        // that, we store the submission of the previous frame here.
        let previous_frame_end = Box::new(sync::now(device.clone())) as Box<GpuFuture>;

        App {
            instance: instance.clone(),
            events_loop,
            surface,
            physical_device_index: physical.index(),
            device,
            queue,
            swapchain,
            images,
            render_pass,
            pipeline,
            dynamic_state,
            framebuffers,
            recreate_swapchain,
            previous_frame_end,
            done: false,
            start_time: std::time::Instant::now(),
            frame_data: FrameData {
                image_num: None,
                acquire_future: None,
                vertex_buffers: vec![],
                command_buffer: None,
            },
        }
    }

    fn run(&mut self) {
        let mut frame_counter = 0;

        loop {
            self.draw_frame();

            frame_counter += 1;

            if self.done {
                break;
            }
        }

        let fps = (frame_counter as f32) / get_elapsed(self.start_time);
        println!("FPS: {}", fps);
    }

    fn draw_frame(&mut self) {
        self.frame_data = FrameData {
            image_num: None,
            acquire_future: None,
            vertex_buffers: vec![],
            command_buffer: None,
        };

        self.free_unused_resources();

        // Whenever the window resizes we need to recreate everything dependent on the window size.
        // In this example that includes the swapchain, the framebuffers and the dynamic state viewport.
        if self.recreate_swapchain {
            self.create_new_swapchain();
            self.recreate_swapchain = false;
        }

        // Before we can draw on the output, we have to *acquire* an image from the swapchain. If
        // no image is available (which happens if you submit draw commands too quickly), then the
        // function will block.
        // This operation returns the index of the image that we are allowed to draw upon.
        //
        // This function can block if no image is available. The parameter is an optional timeout
        // after which the function call will return an error.
        // self.acquire_next_image();

        // We now create a buffer that will store the shape of our triangle.
        self.create_vertex_buffers();

        // In order to draw, we have to build a *command buffer*. The command buffer object holds
        // the list of commands that are going to be executed.
        //
        // Building a command buffer is an expensive operation (usually a few hundred
        // microseconds), but it is known to be a hot path in the driver and is expected to be
        // optimized.
        //
        // Note that we have to pass a queue family when we create the command buffer. The command
        // buffer will only be executable on that given queue family.
        self.create_command_buffer();

        self.submit_and_check();

        // Note that in more complex programs it is likely that one of `acquire_next_image`,
        // `command_buffer::submit`, or `present` will block for some time. This happens when the
        // GPU's queue is full and the driver has to wait until the GPU finished some work.
        //
        // Unfortunately the Vulkan API doesn't provide any way to not wait or to detect when a
        // wait would happen. Blocking may be the desired behavior, but if you don't want to
        // block you should spawn a separate thread dedicated to submissions.

        // Handling the window events in order to close the program when the user wants to close
        // it.
        let mut done = false;
        let mut recreate_swapchain = false;
        self.events_loop.poll_events(|ev| match ev {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => done = true,
            Event::WindowEvent {
                event: WindowEvent::Resized(_),
                ..
            } => recreate_swapchain = true,
            _ => (),
        });

        // for avoiding problems with borrow checker
        self.recreate_swapchain = recreate_swapchain;
        self.done = done
    }

    fn create_command_buffer(&mut self) {
        let clear_values = vec![[0.2, 0.2, 0.2, 1.0].into()];

        let command_buffer = AutoCommandBufferBuilder::primary_one_time_submit(
            self.device.clone(),
            self.queue.family(),
        )
        .unwrap()
        // Before we can draw, we have to *enter a render pass*. There are two methods to do
        // this: `draw_inline` and `draw_secondary`. The latter is a bit more advanced and is
        // not covered here.
        //
        // The third parameter builds the list of values to clear the attachments with. The API
        // is similar to the list of attachments when building the framebuffers, except that
        // only the attachments that use `load: Clear` appear in the list.
        .begin_render_pass(
            self.framebuffers[self.frame_data.image_num.expect(
                "
---------------------------------------------------------------------------------------------
->  When trying to create the command buffer, found that frame_data.image_num is None.
->  Perhaps acquire_next_image was not called.
---------------------------------------------------------------------------------------------
                ")].clone(),
            false,
            clear_values,
        )
        .unwrap()
        // We are now inside the first subpass of the render pass. We add a draw command.
        //
        // The last two parameters contain the list of resources to pass to the shaders.
        // Since we used an `EmptyPipeline` object, the objects have to be `()`.
        .draw(
            self.pipeline.clone(),
            &self.dynamic_state,
            self.frame_data.vertex_buffers[0].clone(),
            (),
            (),
        )
        .unwrap()
        // We are now inside the first subpass of the render pass. We add a draw command.
        //
        // The last two parameters contain the list of resources to pass to the shaders.
        // Since we used an `EmptyPipeline` object, the objects have to be `()`.
        .draw(
            self.pipeline.clone(),
            &self.dynamic_state,
            self.frame_data.vertex_buffers[1].clone(),
            (),
            (),
        )
        .unwrap()
        // We leave the render pass by calling `draw_end`. Note that if we had multiple
        // subpasses we could have called `next_inline` (or `next_secondary`) to jump to the
        // next subpass.
        .end_render_pass()
        .unwrap()
        // Finish building the command buffer by calling `build`.
        .build()
        .unwrap();

        self.frame_data.command_buffer = Some(command_buffer);
    }

    fn submit_and_check(&mut self) {
        let future = self
            .frame_data
            .acquire_future
            .take()
            .expect("Acquire_future is None!")
            .then_execute(
                self.queue.clone(),
                self.frame_data.command_buffer.take().expect("Command_buffer is None!"),
            )
            .unwrap()
            // The color output is now expected to contain our triangle. But in order to show it on
            // the screen, we have to *present* the image by calling `present`.
            //
            // This function does not actually present the image immediately. Instead it submits a
            // present command at the end of the queue. This means that it will only be presented once
            // the GPU has finished executing the command buffer that draws the triangle.
            .then_swapchain_present(
                self.queue.clone(),
                self.swapchain.clone(),
                self.frame_data.image_num.expect("Image_num is None!"),
            )
            .then_signal_fence_and_flush();

        match future {
            Ok(future) => {
                self.previous_frame_end = Box::new(future) as Box<_>;
            }
            Err(FlushError::OutOfDate) => {
                self.recreate_swapchain = true;
                self.previous_frame_end = Box::new(sync::now(self.device.clone())) as Box<_>;
            }
            Err(e) => {
                println!("{:?}", e);
                self.previous_frame_end = Box::new(sync::now(self.device.clone())) as Box<_>;
            }
        }
    }

    fn create_vertex_buffers(&mut self)  {
        let elapsed = get_elapsed(self.start_time);
        let vertex_buffer = {
            CpuAccessibleBuffer::from_iter(
                self.device.clone(),
                BufferUsage::all(),
                [
                    Vertex {
                        position: [elapsed.sin(), elapsed.cos()],
                    },
                    Vertex {
                        position: [(elapsed + 0.5).sin(), (elapsed + 0.5).cos()],
                    },
                    Vertex {
                        position: [(elapsed + 1.0).sin(), (elapsed + 1.0).cos()],
                    },
                ]
                .iter()
                .cloned(),
            )
            .unwrap()
        };

        let vertex_buffer2 = {
            CpuAccessibleBuffer::from_iter(
                self.device.clone(),
                BufferUsage::all(),
                [
                    Vertex {
                        position: [elapsed.cos(), elapsed.sin()],
                    },
                    Vertex {
                        position: [(elapsed + 0.5).cos(), (elapsed + 0.5).sin()],
                    },
                    Vertex {
                        position: [(elapsed + 1.0).cos(), (elapsed + 1.0).sin()],
                    },
                ]
                .iter()
                .cloned(),
            )
            .unwrap()
        };

        self.frame_data.vertex_buffers = vec![vertex_buffer, vertex_buffer2];
    }

    fn free_unused_resources(&mut self) {
        // It is important to call this function from time to time, otherwise resources will keep
        // accumulating and you will eventually reach an out of memory error.
        // Calling this function polls various fences in order to determine what the GPU has
        // already processed, and frees the resources that are no longer needed.
        self.previous_frame_end.cleanup_finished();
    }

    fn create_new_swapchain(&mut self) {
        // Get the new dimensions of the window.
        let dimensions = if let Some(dimensions) = self.surface.window().get_inner_size() {
            let dimensions: (u32, u32) = dimensions
                .to_physical(self.surface.window().get_hidpi_factor())
                .into();
            [dimensions.0, dimensions.1]
        } else {
            return;
        };

        let tuple = match self.swapchain.recreate_with_dimension(dimensions) {
            Ok(r) => r,
            // This error tends to happen when the user is manually resizing the window.
            // Simply restarting the loop is the easiest way to fix this issue.
            Err(SwapchainCreationError::UnsupportedDimensions) => return,
            Err(err) => panic!("{:?}", err),
        };

        let new_swapchain: Arc<Swapchain<Window>> = tuple.0;
        let new_images: Vec<Arc<SwapchainImage<Window>>> = tuple.1;

        self.swapchain = new_swapchain;
        // Because framebuffers contains an Arc on the old swapchain, we need to
        // recreate framebuffers as well.
        self.framebuffers = window_size_dependent_setup(
            &new_images,
            self.render_pass.clone(),
            &mut self.dynamic_state,
        );
    }

    fn acquire_next_image(&mut self) {
        let (image_num, acquire_future) =
            match swapchain::acquire_next_image(self.swapchain.clone(), None) {
                Ok(r) => r,
                Err(AcquireError::OutOfDate) => {
                    self.recreate_swapchain = true;
                    return;
                }
                Err(err) => panic!("{:?}", err),
            };

        self.frame_data.image_num = Some(image_num);
        self.frame_data.acquire_future = Some(acquire_future);
    }
}

fn get_instance() -> Arc<Instance> {
    // When we create an instance, we have to pass a list of extensions that we want to enable.
    //
    // All the window-drawing functionalities are part of non-core extensions that we need
    // to enable manually. To do so, we ask the `vulkano_win` crate for the list of extensions
    // required to draw to a window.
    let extensions = vulkano_win::required_extensions();

    // Now creating the instance.
    Instance::new(None, &extensions, None).unwrap()
}

fn get_physical_device(instance: &Arc<Instance>) -> PhysicalDevice {
    // In a real application, there are three things to take into consideration:
    //
    // - Some devices may not support some of the optional features that may be required by your
    //   application. You should filter out the devices that don't support your app.
    //
    // - Not all devices can draw to a certain surface. Once you create your window, you have to
    //   choose a device that is capable of drawing to it.
    //
    // - You probably want to leave the choice between the remaining devices to the user.
    //
    // For the sake of the example we are just going to use the first device, which should work
    // most of the time.
    PhysicalDevice::enumerate(&instance).next().unwrap()
}

fn get_device_and_queues(
    physical: PhysicalDevice,
    surface: Arc<Surface<Window>>,
) -> (Arc<Device>, vulkano::device::QueuesIter) {
    // The next step is to choose which GPU queue will execute our draw commands.
    //
    // Devices can provide multiple queues to run commands in parallel (for example a draw queue
    // and a compute queue), similar to CPU threads. This is something you have to have to manage
    // manually in Vulkan.
    //
    // In a real-life application, we would probably use at least a graphics queue and a transfers
    // queue to handle data transfers in parallel. In this example we only use one queue.
    //
    // We have to choose which queues to use early on, because we will need this info very soon.
    let queue_family = physical
        .queue_families()
        .find(|&q| {
            // We take the first queue that supports drawing to our window.
            q.supports_graphics() && surface.is_supported(q).unwrap_or(false)
        })
        .unwrap();

    // Now initializing the device. This is probably the most important object of Vulkan.
    //
    // We have to pass five parameters when creating a device:
    //
    // - Which physical device to connect to.
    //
    // - A list of optional features and extensions that our program needs to work correctly.
    //   Some parts of the Vulkan specs are optional and must be enabled manually at device
    //   creation. In this example the only thing we are going to need is the `khr_swapchain`
    //   extension that allows us to draw to a window.
    //
    // - A list of layers to enable. This is very niche, and you will usually pass `None`.
    //
    // - The list of queues that we are going to use. The exact parameter is an iterator whose
    //   items are `(Queue, f32)` where the floating-point represents the priority of the queue
    //   between 0.0 and 1.0. The priority of the queue is a hint to the implementation about how
    //   much it should prioritize queues between one another.
    //
    // The list of created queues is returned by the function alongside with the device.
    let device_ext = DeviceExtensions {
        khr_swapchain: true,
        ..DeviceExtensions::none()
    };
    // let (device, mut queues) = Device::new(physical, physical.supported_features(), &device_ext,
    //     [(queue_family, 0.5)].iter().cloned()).unwrap();
    Device::new(
        physical,
        physical.supported_features(),
        &device_ext,
        [(queue_family, 0.5)].iter().cloned(),
    )
    .unwrap()
}

fn create_swapchain_and_images(
    physical: PhysicalDevice,
    surface: Arc<Surface<Window>>,
    device: Arc<Device>,
    queue: Arc<Queue>,
) -> (Arc<Swapchain<Window>>, Vec<Arc<SwapchainImage<Window>>>) {
    // Before we can draw on the surface, we have to create what is called a swapchain. Creating
    // a swapchain allocates the color buffers that will contain the image that will ultimately
    // be visible on the screen. These images are returned alongside with the swapchain.

    // Querying the capabilities of the surface. When we create the swapchain we can only
    // pass values that are allowed by the capabilities.
    let caps = surface.capabilities(physical).unwrap();

    let usage = caps.supported_usage_flags;

    // The alpha mode indicates how the alpha value of the final image will behave. For example
    // you can choose whether the window will be opaque or transparent.
    let alpha = caps.supported_composite_alpha.iter().next().unwrap();

    // Choosing the internal format that the images will have.
    let format = caps.supported_formats[0].0;

    // The dimensions of the window, only used to initially setup the swapchain.
    // NOTE:
    // On some drivers the swapchain dimensions are specified by `caps.current_extent` and the
    // swapchain size must use these dimensions.
    // These dimensions are always the same as the window dimensions
    //
    // However other drivers dont specify a value i.e. `caps.current_extent` is `None`
    // These drivers will allow anything but the only sensible value is the window dimensions.
    //
    // Because for both of these cases, the swapchain needs to be the window dimensions, we just use that.
    let initial_dimensions = if let Some(dimensions) = surface.window().get_inner_size() {
        // convert to physical pixels
        let dimensions: (u32, u32) = dimensions
            .to_physical(surface.window().get_hidpi_factor())
            .into();
        [dimensions.0, dimensions.1]
    } else {
        // The window no longer exists so exit the application.
        panic!("The window no longer exists! this should not happen.");
    };

    // Please take a look at the docs for the meaning of the parameters we didn't mention.
    Swapchain::new(
        device.clone(),
        surface.clone(),
        caps.min_image_count,
        format,
        initial_dimensions,
        1,
        usage,
        &queue,
        SurfaceTransform::Identity,
        alpha,
        PresentMode::Fifo,
        true,
        None,
    )
    .unwrap()
}

/// This method is called once during initialization, then again whenever the window is resized
fn window_size_dependent_setup(
    images: &[Arc<SwapchainImage<Window>>],
    render_pass: Arc<RenderPassAbstract + Send + Sync>,
    dynamic_state: &mut DynamicState,
) -> Vec<Arc<FramebufferAbstract + Send + Sync>> {
    let dimensions = images[0].dimensions();

    let viewport = Viewport {
        origin: [0.0, 0.0],
        dimensions: [dimensions[0] as f32, dimensions[1] as f32],
        depth_range: 0.0..1.0,
    };
    dynamic_state.viewports = Some(vec![viewport]);

    images
        .iter()
        .map(|image| {
            Arc::new(
                Framebuffer::start(render_pass.clone())
                    .add(image.clone())
                    .unwrap()
                    .build()
                    .unwrap(),
            ) as Arc<FramebufferAbstract + Send + Sync>
        })
        .collect::<Vec<_>>()
}

pub fn get_elapsed(start: std::time::Instant) -> f32 {
    start.elapsed().as_secs() as f32 + start.elapsed().subsec_nanos() as f32 / 1_000_000_000.0
}
