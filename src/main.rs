#[macro_use]
extern crate glium;

extern crate glium_sdl2;
extern crate sdl2;

extern crate nalgebra;
extern crate byteorder;
extern crate lmdb_rs as lmdb;
extern crate uuid;
extern crate rmp_serde;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate tempdir;
extern crate noise;

mod world;
mod shaders;
mod database;
mod chunk;

fn main() {
    use glium_sdl2::DisplayBuild;

    let sdl_context = sdl2::init().unwrap();
    let video_subsystem = sdl_context.video().unwrap();

    let display = video_subsystem.window("My window", 800, 600)
        .resizable()
        .build_glium()
        .unwrap();

    sdl_context.mouse().set_relative_mouse_mode(true); // FPS mouse

    let mut world = world::World::new(&display);

    let mut running = true;
    let mut event_pump = sdl_context.event_pump().unwrap();

    while running {

        for event in event_pump.poll_iter() {
            use sdl2::event::Event;

            match event {
                Event::Quit { .. } => {
                    running = false;
                },
                ev => world.process_input(&ev),
            }
        }

        world.step();

        let mut target = display.draw();

        world.render(&display, &mut target);

        target.finish().unwrap();
    }
}
