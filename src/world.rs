use glium;
use glium_sdl2;
use sdl2;
use std::collections::HashMap;
use nalgebra as na;
use std::path::Path;
use shaders;
use database;
use byteorder;
use uuid;
use std::cell::{RefCell};

use chunk::{Chunk, CHUNK_SIDE};

type Vector3 = na::Vector3<f32>;


struct Player {
    position: Vector3,
    camera_pitch: f32,
    camera_yaw: f32,
}

pub struct World {
    keystate: HashMap<sdl2::keyboard::Keycode, bool>,
    chunks: HashMap<(i32, i32, i32), RefCell<Chunk>>,
    program: glium::Program,
    player: Player,
    database: database::TypedDatabase<World>,
}

impl World {
    pub fn new(display: &glium_sdl2::SDL2Facade) -> World {
        let w = World {
            keystate: HashMap::new(),
            chunks: HashMap::new(),
            program: shaders::load_shaders(&display),
            player: Player {
                position: Vector3::new(0.0, 0.0, 0.0),
                camera_pitch: 0.0,
                camera_yaw: 0.0
            },
            database: database::TypedDatabase::new(database::Database::create_env(Path::new("lmdb")))
        };

        w
    }

    pub fn get_perspective(&self) -> na::Matrix4<f32> {

        /*
        let aspect_ratio = 1024.0 / 768.0;

        let fov: f32 = 3.141592 / 2.0;
        let zfar = 1024.0;
        let znear = 0.1;

        let f = 1.0 / (fov / 2.0).tan();

        // note: remember that this is column-major, so the lines of code are actually columns
        [
            [f / aspect_ratio,    0.0,              0.0              ,   0.0],
            [         0.0    ,     f ,              0.0              ,   0.0],
            [         0.0    ,    0.0,  (zfar+znear)/(znear-zfar)    ,  -1.0],
            [         0.0    ,    0.0,  (2.0*zfar*znear)/(znear-zfar),   0.0],
        ]*/
        let projection = na::Perspective3::new(800.0 / 600.0, 3.14 / 2.0, 0.1, 1000.0)
                .to_homogeneous();
        projection
    }

    pub fn get_view(&self) -> na::Matrix4<f32> {
        const CAMERA_RADIUS: f32 = 1.0;

        let rot = na::Rotation::from_euler_angles(0.0, self.player.camera_pitch, 0.0);
        let rot = na::Rotation::from_euler_angles(0.0, 0.0, self.player.camera_yaw) * rot;
        //let rot = na::Rotation::from_euler_angles(0.0, self.camera_pitch, self.camera_yaw);

        let view =
            na::Isometry3::look_at_rh(
                &na::Point::from_coordinates(rot * na::Vector3::new(-CAMERA_RADIUS, 0.0, 0.0)),
                &na::Point::from_coordinates(na::Vector3::new(0.0, 0.0, 0.0)),
                &na::Vector3::z(),
            ).to_homogeneous();
        let view = view * na::Translation3::from_vector(-self.player.position).to_homogeneous();
        view
    }

    fn chunk_uuid(&self, position: (i32, i32, i32)) -> uuid::Uuid {
        use std::io::Cursor;
        use byteorder::WriteBytesExt;
        let mut buf = [0u8; 3*4];
        {
            let mut writer = Cursor::new(&mut buf[..]);
            writer.write_i32::<byteorder::BigEndian>(position.0).unwrap();
            writer.write_i32::<byteorder::BigEndian>(position.1).unwrap();
            writer.write_i32::<byteorder::BigEndian>(position.2).unwrap();
        }
        uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, &buf)
    }

    pub fn render(&mut self, display: &glium_sdl2::SDL2Facade, frame: &mut glium::Frame) {

        use glium::Surface;

        let uniforms = uniform! {
            persp_matrix: *self.get_perspective().as_ref(),
            view_matrix: *self.get_view().as_ref(),
        };

        let params = glium::DrawParameters {
            depth: glium::Depth {
                test: glium::DepthTest::IfLess,
                write: true,
                .. Default::default()
            },
            .. Default::default()
        };

        frame.clear_color_and_depth((0.0, 0.0, 0.0, 0.0), 1.0);
        for (_, chunk) in self.chunks.iter_mut() {

            let mut chunk = chunk.borrow_mut();
            if chunk.vertex_buffer.is_none() || chunk.index_buffer.is_none() {
                chunk.update_buffers(display);
            }

            if let (Some(vb), Some(ib)) = (&chunk.vertex_buffer, &chunk.index_buffer) {
                frame.draw(vb, ib, &self.program, &uniforms, &params).unwrap();
            }
        }
    }

    pub fn step(&mut self) {

        // run pending db ops
        self.database.tick()(self);

        // remove chunks waiting for removal
        {
            for (_, chunk) in self.chunks.iter() {
                let chunk: &Chunk = &chunk.borrow();
                if chunk.remove {
                    println!("dropping {:?}", chunk.position);
                    self.database.put(chunk);
                }
            }
        }
        self.chunks.retain(|_, c| c.borrow().remove == false);

        // move player
        let mut movement = Vector3::new(0.0, 0.0, 0.0);
        if *self.keystate.get(&sdl2::keyboard::Keycode::W).unwrap_or(&false) {
            movement.x += 1.0;
        }
        else if *self.keystate.get(&sdl2::keyboard::Keycode::S).unwrap_or(&false) {
            movement.x -= 1.0;
        }
        if *self.keystate.get(&sdl2::keyboard::Keycode::A).unwrap_or(&false) {
            movement.y += 1.0;
        }
        else if *self.keystate.get(&sdl2::keyboard::Keycode::D).unwrap_or(&false) {
            movement.y -= 1.0;
        }
        let dt = 0.1;
        let movement = na::Rotation::from_euler_angles(0.0, 0.0, self.player.camera_yaw) * movement;
        self.player.position += movement * dt;

        // request missing chunks around the player
        const REQUEST_RADIUS: i32 = 5;

        for (_, chunk) in self.chunks.iter_mut() {
            chunk.borrow_mut().remove = true;
        }

        for z in -REQUEST_RADIUS..=REQUEST_RADIUS {
            for y in -REQUEST_RADIUS..=REQUEST_RADIUS {
                for x in -REQUEST_RADIUS..=REQUEST_RADIUS {
                    let key = (
                        (self.player.position.x as i32) / CHUNK_SIDE + x,
                        (self.player.position.y as i32) / CHUNK_SIDE + y,
                        (self.player.position.z as i32) / CHUNK_SIDE + z
                    );

                    if self.chunks.contains_key(&key) {
                        self.chunks.get_mut(&key).unwrap().borrow_mut().remove = false;
                    }
                    else {
                        // request chunk
                        let chunk_id = self.chunk_uuid(key);
                        self.database.get_ctx(chunk_id, move |world: &mut World, chunk: Option<Chunk>|{
                            if let Some(chunk) = chunk {
                                println!("loading {:?}", key);
                                world.chunks.insert(chunk.position, RefCell::new(chunk));
                            }
                            else {
                                println!("generating {:?}", key);
                                world.generate_chunk(key);
                            }
                        });
                    }
                }
            }
        }
    }

    fn generate_chunk(&mut self, position: (i32, i32, i32)) {
        let chunk = Chunk::generate(position, self.chunk_uuid(position), 0);
        self.chunks.insert(chunk.position, RefCell::new(chunk));
    }

    pub fn process_input(&mut self, event: &sdl2::event::Event) {
        use sdl2::event::Event::*;

        match event {
            &KeyDown {keycode: Some(x), ..} => {*self.keystate.entry(x).or_insert(false) = true;},
            &KeyUp {keycode: Some(x), ..} => {*self.keystate.entry(x).or_insert(false) = false;},
            &MouseMotion {xrel, yrel, ..} => {
                self.player.camera_pitch += yrel as f32 / 500.0;
                self.player.camera_yaw -= xrel as f32 / 500.0;
                //println!("{} {}", self.player.camera_pitch, self.player.camera_yaw);
            }
            _ => {}
        }
    }
}
impl Drop for World {
    fn drop(&mut self) {
        // persist all chunks in the database
        let chunks = self.chunks.drain().map(|(_,v)| v).map(|c| c.into_inner());
        self.database.put_many(chunks).unwrap();
        self.database.flush();
    }
}
