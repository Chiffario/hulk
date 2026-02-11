use memfd::Memfd;
use std::{
    fmt::Display,
    io::{Read, Seek, SeekFrom, Write},
    os::{
        fd::BorrowedFd,
        unix::{
            io::AsRawFd,
            net::UnixListener,
        },
    },
    path::Path,
};
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle, delegate_noop,
    protocol::{wl_output, wl_registry},
};
use wayland_protocols_wlr::{
    self,
    gamma_control::v1::client::{
            zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1,
            zwlr_gamma_control_v1::{self, ZwlrGammaControlV1},
        },
};

use crate::{SOCKET_PATH, message::IpcMessage};

struct AppData {
    outputs: Vec<wl_output::WlOutput>,
    gamma_manager: Option<ZwlrGammaControlManagerV1>,
    gamma_control: Option<ZwlrGammaControlV1>,
    ramp_size: u32,
    gamma_applied: bool,
}
impl AppData {
    fn new() -> Self {
        Self {
            outputs: Vec::new(),
            gamma_manager: None,
            gamma_control: None,
            ramp_size: 0,
            gamma_applied: false,
        }
    }

    fn reset_gamma_control(&mut self, output_idx: usize, queue_handle: &QueueHandle<AppData>) -> Result<(), Box<dyn std::error::Error>> {
        // Destroy current gamma control
        let output = self.outputs.get(output_idx).ok_or("Invalid output")?;
        let control = self.gamma_control.take();
        if let Some(c) = control {
            c.destroy();
        }

        // Get gamma control for the output
        let gamma_manager = self.gamma_manager.as_ref().ok_or("No gamma manager")?;
        let gamma_control = gamma_manager.get_gamma_control(output, queue_handle, ());
        self.gamma_control = Some(gamma_control);
        Ok(())
    }
    fn prepare_data(&self, gamma: f32) -> Result<Vec<u8>, Errors> {
        if self.ramp_size == 0 {
            eprintln!("Failed to get gamma ramp size");
            return Err(Errors::GammaRampError);
        }

        // Create gamma ramp
        let (red, green, blue) = create_gamma_ramp(self.ramp_size as usize, gamma);

        // Prepare gamma table data
        let mut data = Vec::new();
        for i in 0..self.ramp_size as usize {
            data.extend_from_slice(&red[i].to_ne_bytes());
        }
        for i in 0..self.ramp_size as usize {
            data.extend_from_slice(&green[i].to_ne_bytes());
        }
        for i in 0..self.ramp_size as usize {
            data.extend_from_slice(&blue[i].to_ne_bytes());
        }
        Ok(data)
    }

    fn set_gamma(&mut self,
        gamma: f32,
        output_idx: usize,
        queue_handle: &QueueHandle<AppData>,
        event_queue: &mut EventQueue<AppData>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.reset_gamma_control(output_idx, queue_handle)?;

        // Wait for gamma_size event
        event_queue.roundtrip(self)?;

        let data = self.prepare_data(gamma)?;
        let mfd = prepare_fd(&data)?;
        if let Some(ref control) = self.gamma_control {
            // SAFETY: mfd should be valid throughout the process
            control.set_gamma(unsafe { BorrowedFd::borrow_raw(mfd.as_raw_fd()) });
            println!("Setting gamma to {} for output {}", gamma, output_idx);
        }

        // Wait for the compositor to process the gamma change
        event_queue.roundtrip(self)?;
        Ok(())

    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for AppData {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match &interface[..] {
                "wl_output" => {
                    let output = registry.bind::<wl_output::WlOutput, _, _>(name, version, qh, ());
                    state.outputs.push(output);
                }
                "zwlr_gamma_control_manager_v1" => {
                    let manager =
                        registry.bind::<ZwlrGammaControlManagerV1, _, _>(name, version, qh, ());
                    state.gamma_manager = Some(manager);
                }
                _ => {}
            }
        }
    }
}

delegate_noop!(AppData: ignore wl_output::WlOutput);
delegate_noop!(AppData: ignore ZwlrGammaControlManagerV1);

impl Dispatch<ZwlrGammaControlV1, ()> for AppData {
    fn event(
        state: &mut Self,
        _: &ZwlrGammaControlV1,
        event: zwlr_gamma_control_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_gamma_control_v1::Event::GammaSize { size } => {
                println!("Gamma ramp size: {}", size);
                state.ramp_size = size;
            }
            zwlr_gamma_control_v1::Event::Failed => {
                eprintln!("Failed to set gamma");
                state.gamma_applied = true;
            }
            _ => {}
        }
    }
}

fn create_gamma_ramp(size: usize, gamma: impl Into<f32>) -> (Vec<u16>, Vec<u16>, Vec<u16>) {
    let gamma = gamma.into();
    let mut red = Vec::with_capacity(size);
    let mut green = Vec::with_capacity(size);
    let mut blue = Vec::with_capacity(size);

    for i in 0..size {
        let value = (i as f32 / (size - 1) as f32).powf(1.0 / gamma);
        let scaled = (value * 65535.0) as u16;
        red.push(scaled);
        green.push(scaled);
        blue.push(scaled);
    }

    (red, green, blue)
}

pub fn wayland_loop(
    initial_index: usize,
    initial_gamma: f32,
) -> Result<(), Box<dyn std::error::Error>> {
    let _ = std::fs::remove_file(SOCKET_PATH);
    // Create basic connections
    let conn = Connection::connect_to_env()?;
    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();

    // Initialize a registry
    let display = conn.display();
    let _ = display.get_registry(&qh, ());
    let mut state = AppData::new();

    // Get globals
    event_queue.roundtrip(&mut state)?;

    if state.outputs.is_empty() {
        eprintln!("No outputs found");
        return Ok(());
    }

    let Some(_gamma_manager) = &state.gamma_manager else {
        eprintln!("Compositor doesn't support wlr-gamma-control protocol");
        return Ok(());
    };

    if initial_index >= state.outputs.len() {
        eprintln!("Output index {} not available", initial_index);
        return Ok(());
    }
    state.set_gamma(initial_gamma, initial_index, &qh, &mut event_queue)?;

    let path = SOCKET_PATH;
    std::fs::create_dir_all(Path::new(path).parent().unwrap())?;
    let stream = UnixListener::bind(path)?;
    println!("Connected to stream {:?}", stream.local_addr()?);

    let mut buffer = String::new();
    for mut msg in stream.incoming().flatten() {
        let _ = msg.read_to_string(&mut buffer);
        let message: IpcMessage = ron::from_str(&buffer)?;
        println!("Received message: {message:?}");
        buffer.clear();
        let gamma = message.gamma.unwrap_or(initial_gamma);
        let output = message.output.unwrap_or(initial_index);
        state.set_gamma(gamma, output, &qh, &mut event_queue)?;
    }
    Ok(())
}

#[derive(Debug)]
enum Errors {
    GammaRampError,
}
impl Display for Errors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Errors::GammaRampError => write!(f, "Failed to get gamma ramp size"),
        }
    }
}
impl std::error::Error for Errors {}


fn prepare_fd(data: &[u8]) -> Result<Memfd, Box<dyn std::error::Error>> {
    let opts = memfd::MemfdOptions::default().allow_sealing(true);
    let mfd = opts.create(format!("gamma-ramp{}", data[0]))?;

    // scope trick to drop the file reference ASAP
    {
        let mut file = mfd.as_file();
        file.seek(SeekFrom::Start(0))?;
        file.write_all(data)?;
        file.seek(SeekFrom::Start(0))?;
        file.flush()?;
    }

    // Wayland expects exact-size FDs, sealing prevents any bs from happening
    let seals = memfd::SealsHashSet::from_iter([
        memfd::FileSeal::SealShrink,
        memfd::FileSeal::SealGrow,
        memfd::FileSeal::SealWrite,
    ]);
    mfd.add_seals(&seals)?;
    Ok(mfd)
}
