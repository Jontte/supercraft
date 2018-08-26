use byteorder;
use chunk::{Chunk, ChunkView, CHUNK_SIDE};
use database;
use fnv::FnvHashMap;
use glium;
use glium_sdl2;
use nalgebra as na;
use sdl2;
use shaders;
use shaders::Vertex;
use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::sync::{
    mpsc::{channel, Receiver, Sender},
    Arc, RwLock,
};
use std::thread;
use uuid;

type Vector3 = na::Vector3<f32>;
type Vector3i = na::Vector3<i32>;

use std::time::Instant;
struct Timer {
    instant: Instant,
}
impl Timer {
    pub fn new() -> Timer {
        Timer {
            instant: Instant::now(),
        }
    }
    pub fn elapsed(&self) -> f64 {
        let duration = self.instant.elapsed();
        duration.as_secs() as f64 + duration.subsec_nanos() as f64 * 1e-9
    }
}

struct Player {
    position: Vector3,
    camera_pitch: f32,
    camera_yaw: f32,
}

pub struct World {
    keystate: FnvHashMap<sdl2::keyboard::Keycode, bool>,
    chunks: FnvHashMap<Vector3i, Arc<RwLock<Chunk>>>,
    chunk_views: FnvHashMap<Vector3i, RefCell<ChunkView>>,
    program: glium::Program,
    player: Player,
    database: database::TypedDatabase<World>,

    to_worker_tx: Sender<ToWorkerMessage>,
    from_worker_rx: Receiver<FromWorkerMessage>,

    frame_counter: u64,
}

enum ToWorkerMessage {
    NewChunk(Vector3i, Arc<RwLock<Chunk>>),
    UpdateChunk(Vector3i),
    DropChunk(Vec<Vector3i>),
    SetPriorityPoint(Vector3i),
    GenerateChunk(Vector3i, u64),
}
enum FromWorkerMessage {
    Mesh(Vector3i, (Vec<Vertex>, Vec<u16>)),
    GenerateFinish(Chunk)
}

impl World {
    pub fn new(display: &glium_sdl2::SDL2Facade) -> World {
        let (to_worker_tx, to_worker_rx) = channel();
        let (from_worker_tx, from_worker_rx) = channel();

        let w = World {
            keystate: FnvHashMap::default(),
            chunks: FnvHashMap::default(),
            chunk_views: FnvHashMap::default(),
            program: shaders::load_shaders(&display),
            player: Player {
                position: Vector3::new(0.0, 0.0, 0.0),
                camera_pitch: 0.0,
                camera_yaw: 0.0,
            },
            database: database::TypedDatabase::new(database::Database::create_env(Path::new(
                "lmdb",
            ))),
            to_worker_tx,
            from_worker_rx,
            frame_counter: 0,
        };

        // spawn worker thread for calculating meshes
        thread::spawn(move || {
            let mut chunks: HashMap<Vector3i, Arc<RwLock<Chunk>>> = HashMap::new();
            let mut update_queue = HashSet::new();
            let mut priority_point = Vector3i::new(0, 0, 0);

            loop {
                {
                    let queue_empty = update_queue.len() == 0;
                    let mut handle_message = |x| {
                        match x {
                            ToWorkerMessage::NewChunk(position, chunk) => {
                                chunks.insert(position, chunk);
                                update_queue.insert(position);

                                let neighbors = [
                                    Vector3i::new(-1, 0, 0),
                                    Vector3i::new(1, 0, 0),
                                    Vector3i::new(0, -1, 0),
                                    Vector3i::new(0, 1, 0),
                                    Vector3i::new(0, 0, -1),
                                    Vector3i::new(0, 0, 1),
                                ];
                                for n in neighbors.iter() {
                                    update_queue.insert(position + n);
                                }
                            }
                            ToWorkerMessage::DropChunk(positions) => {
                                for p in positions {
                                    chunks.remove(&p);
                                }
                                // TODO: need updates during delete?
                                //update_queue.push(position)
                            }
                            ToWorkerMessage::UpdateChunk(position) => {
                                update_queue.insert(position);
                                let neighbors = [
                                    Vector3i::new(-1, 0, 0),
                                    Vector3i::new(1, 0, 0),
                                    Vector3i::new(0, -1, 0),
                                    Vector3i::new(0, 1, 0),
                                    Vector3i::new(0, 0, -1),
                                    Vector3i::new(0, 0, 1),
                                ];
                                for n in neighbors.iter() {
                                    update_queue.insert(position + n);
                                }
                            }
                            ToWorkerMessage::SetPriorityPoint(p) => {
                                priority_point = p;
                            }
                            ToWorkerMessage::GenerateChunk(position, seed) => {
                                let chunk = Chunk::generate(position, seed);
                                from_worker_tx
                                    .send(FromWorkerMessage::GenerateFinish(chunk))
                                    .unwrap();
                            }
                        }
                    };

                    if queue_empty {
                        if let Ok(x) = to_worker_rx.recv() {
                            handle_message(x);
                            // get as many queued messages as possible
                            while let Ok(x) = to_worker_rx.try_recv() {
                                handle_message(x);
                            }
                        } else {
                            // pipe closed? time to stop work
                            return;
                        }
                    } else {
                        while let Ok(x) = to_worker_rx.try_recv() {
                            handle_message(x);
                        }
                    }
                }

                if update_queue.len() == 0 {
                    continue;
                }

                // find elem with highest priority from set
                let position: Vector3i = *update_queue
                    .iter()
                    .min_by_key(|p| {
                        let dist = (*p * CHUNK_SIDE) - priority_point;
                        dist.x.abs() + dist.y.abs() + dist.z.abs()
                    })
                    .unwrap();

                update_queue.remove(&position);

                //let dist = (position * CHUNK_SIDE) - priority_point;
                //let dist = dist.x.abs() + dist.y.abs() + dist.z.abs();

                if let Some(chunk) = chunks.get(&position) {
                    //println!("prio {}, jobs = {}", dist, update_queue.len());
                    let mut chunk = chunk.write().unwrap();
                    let neighbors = [
                        chunks.get(&(chunk.position + Vector3i::new(-1, 0, 0))),
                        chunks.get(&(chunk.position + Vector3i::new(1, 0, 0))),
                        chunks.get(&(chunk.position + Vector3i::new(0, -1, 0))),
                        chunks.get(&(chunk.position + Vector3i::new(0, 1, 0))),
                        chunks.get(&(chunk.position + Vector3i::new(0, 0, -1))),
                        chunks.get(&(chunk.position + Vector3i::new(0, 0, 1))),
                    ];
                    let (vertices, indices) = chunk.update_polymesh(neighbors);
                    from_worker_tx
                        .send(FromWorkerMessage::Mesh(
                            chunk.position,
                            (vertices, indices),
                        ))
                        .unwrap();
                }
            }
        });

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
        let projection =
            na::Perspective3::new(800.0 / 600.0, 3.1415 / 2.0, 0.1, 1000.0).to_homogeneous();
        projection
    }

    pub fn get_view(&self) -> na::Matrix4<f32> {
        const CAMERA_RADIUS: f32 = 1.0;

        let rot = na::Rotation::from_euler_angles(0.0, self.player.camera_pitch, 0.0);
        let rot = na::Rotation::from_euler_angles(0.0, 0.0, self.player.camera_yaw) * rot;
        //let rot = na::Rotation::from_euler_angles(0.0, self.camera_pitch, self.camera_yaw);

        let view = na::Isometry3::look_at_rh(
            &na::Point::from_coordinates(rot * na::Vector3::new(-CAMERA_RADIUS, 0.0, 0.0)),
            &na::Point::from_coordinates(na::Vector3::new(0.0, 0.0, 0.0)),
            &na::Vector3::z(),
        ).to_homogeneous();
        let view = view * na::Translation3::from_vector(-self.player.position).to_homogeneous();
        view
    }

    fn chunk_uuid(&self, position: Vector3i) -> uuid::Uuid {
        use byteorder::WriteBytesExt;
        use std::io::Cursor;
        let mut buf = [0u8; 3 * 4];
        {
            let mut writer = Cursor::new(&mut buf[..]);
            writer
                .write_i32::<byteorder::BigEndian>(position.x)
                .unwrap();
            writer
                .write_i32::<byteorder::BigEndian>(position.y)
                .unwrap();
            writer
                .write_i32::<byteorder::BigEndian>(position.z)
                .unwrap();
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
                ..Default::default()
            },
            ..Default::default()
        };

        frame.clear_color_and_depth((0.0, 0.0, 0.0, 0.0), 1.0);

        // send newly generated meshes to the gpu
        while let Ok(msg) = self.from_worker_rx.try_recv() {
            match msg {
                FromWorkerMessage::Mesh(pos, mesh) => {
                    if let Some(chunk) = self.chunk_views.get_mut(&pos) {
                        chunk.borrow_mut().update_mesh(display, mesh);
                    }
                }
                FromWorkerMessage::GenerateFinish(chunk) => {
                    self.insert_chunk(chunk);
                }
            }
        }

        let camera_rot = na::Rotation::from_euler_angles(0.0, self.player.camera_pitch, 0.0);
        let camera_rot = na::Rotation::from_euler_angles(0.0, 0.0, self.player.camera_yaw) * camera_rot * Vector3::new(1.0, 0.0, 0.0);

        for (pos, chunk) in self.chunk_views.iter() {

            // TODO: move query point backwards to add some uniform margin...
            let chunk_center = pos.map(|x| x as f32 + 0.5) * CHUNK_SIDE as f32;
            let player_to_chunk = chunk_center - self.player.position;
            let cos_a = na::dot(&camera_rot, &player_to_chunk) / player_to_chunk.norm();

            if cos_a < (3.141f32/2.0).cos() && player_to_chunk.norm() > CHUNK_SIDE as f32 {
                continue;
            }

            let chunk = chunk.borrow();
            if let (Some(vb), Some(ib)) = (&chunk.vertex_buffer, &chunk.index_buffer) {
                frame
                    .draw(vb, ib, &self.program, &uniforms, &params)
                    .unwrap();
            }
        }
    }

    pub fn step(&mut self) {
        self.frame_counter += 1;
        // run pending db ops
        {
            //let t = Timer::new();
            self.database.tick()(self);
            //println!("db ops {}", t.elapsed());
        }

        // remove chunks waiting for removal
        if self.frame_counter % (30 * 1) == 0 {
            let keys: Vec<Vector3i> = self
                .chunk_views
                .iter()
                .filter_map(|(k, _v)| {
                    let dist = k * CHUNK_SIDE - self.player.position.map(|x| x as i32);
                    let dist = dist.x.abs() + dist.y.abs() + dist.z.abs();

                    if dist > CHUNK_SIDE * 20 {
                        Some(k.clone())
                    } else {
                        None
                    }
                })
                .collect();
            if keys.len() > 0 {
                let mut puts: Vec<Arc<RwLock<Chunk>>> = Vec::new();
                //let t = Timer::new();
                for key in keys.iter() {
                    self.chunk_views.remove(&key).unwrap();
                    if let Some(chunk) = self.chunks.remove(&key) {
                        //puts.push(chunk.read().unwrap().p.clone());
                        puts.push(chunk.clone());
                    }
                }
                //println!("p1 {} s", t.elapsed());
                self.to_worker_tx
                    .send(ToWorkerMessage::DropChunk(keys))
                    .unwrap();

                //println!("p2 {} s", t.elapsed());
                //let putslen = puts.len();
                if puts.len() > 0 {
                    self.database
                        .put_many_async_fn(move || {
                            puts.iter().map(|x| x.read().unwrap().clone()).collect()
                        })
                        .unwrap();
                    //println!("world put of {} took {} s", putslen, t.elapsed());
                }
            }
        }

        // move player
        let mut movement = Vector3::new(0.0, 0.0, 0.0);
        if *self
            .keystate
            .get(&sdl2::keyboard::Keycode::W)
            .unwrap_or(&false)
        {
            movement.x += 1.0;
        } else if *self
            .keystate
            .get(&sdl2::keyboard::Keycode::S)
            .unwrap_or(&false)
        {
            movement.x -= 1.0;
        }
        if *self
            .keystate
            .get(&sdl2::keyboard::Keycode::A)
            .unwrap_or(&false)
        {
            movement.y += 1.0;
        } else if *self
            .keystate
            .get(&sdl2::keyboard::Keycode::D)
            .unwrap_or(&false)
        {
            movement.y -= 1.0;
        }
        let dt = 0.1 * 10.0;

        let rot = na::Rotation::from_euler_angles(0.0, self.player.camera_pitch, 0.0);
        let rot = na::Rotation::from_euler_angles(0.0, 0.0, self.player.camera_yaw) * rot;

        let movement = rot * movement;

        self.player.position += movement * dt;

        if self.frame_counter % (30 * 5) == 0 {
            self.to_worker_tx
                .send(ToWorkerMessage::SetPriorityPoint(
                    self.player.position.map(|x| x as i32),
                ))
                .unwrap();
        }

        // request missing chunks around the player
        if self.frame_counter % (30) == 0 {
            //let t = Timer::new();
            const REQUEST_RADIUS: i32 = 5;

            // TODO: request based on chunkviews ....
            let mut requests = Vec::new();
            let mut positions = Vec::new();
            for z in -REQUEST_RADIUS..=REQUEST_RADIUS {
                for y in -REQUEST_RADIUS..=REQUEST_RADIUS {
                    for x in -REQUEST_RADIUS..=REQUEST_RADIUS {
                        let key: Vector3i = self.player.position.map(|x| x as i32);
                        let key = key / CHUNK_SIDE + Vector3i::new(x, y, z);

                        let do_request_chunk = !self.chunk_views.contains_key(&key);

                        if do_request_chunk {
                            let chunk_id = self.chunk_uuid(key);

                            // add a placeholder chunk while the actual chunk is being retrieved
                            self.chunk_views
                                .insert(key, RefCell::new(ChunkView::new(key)));
                            positions.push(key);
                            requests.push(chunk_id);
                        }
                    }
                }
            }
            if requests.len() > 0 {
                //println!("requesting {} from db", requests.len());
                self.database.get_ctx(
                    requests.as_slice(),
                    move |world: &mut World, chunks: Vec<Option<Chunk>>| {
                        //let mut generating = 0.0f64;

                        for (i, chunk) in chunks.into_iter().enumerate() {
                            if let Some(chunk) = chunk {
                                //println!("loading {:?}", key);
                                world.insert_chunk(chunk);
                            } else {
                                //println!("generating {:?}", key);
                                //let t = Timer::new();
                                world.to_worker_tx
                                    .send(ToWorkerMessage::GenerateChunk(positions[i], 0))
                                    .unwrap();
                                //generating += t.elapsed();
                            }
                        }
                        //println!("generated in {} s", generating);
                    },
                );
                //println!("request of {} took {} s", requests.len(), t.elapsed());
            }
        }
    }
    fn insert_chunk(&mut self, mut chunk: Chunk) {
        // view is assumed to exist!
        if self.chunk_views.get(&chunk.position).is_none() {
            return;
        }
        let pos = chunk.position;
        chunk.uuid = self.chunk_uuid(pos);
        let chunk = Arc::new(RwLock::new(chunk));
        self.chunks.insert(pos, chunk.clone());

        self.to_worker_tx
            .send(ToWorkerMessage::NewChunk(pos, chunk))
            .unwrap();
    }

    pub fn process_input(&mut self, event: &sdl2::event::Event) {
        use sdl2::event::Event::*;

        match event {
            &KeyDown {
                keycode: Some(x), ..
            } => {
                *self.keystate.entry(x).or_insert(false) = true;
            }
            &KeyUp {
                keycode: Some(x), ..
            } => {
                *self.keystate.entry(x).or_insert(false) = false;
            }
            &MouseMotion { xrel, yrel, .. } => {
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
        let chunks = self
            .chunks
            .drain()
            .map(|(_, v)| v)
            .map(|c| c.read().unwrap().clone());
        self.database.put_many(chunks).unwrap();
        self.database.flush();
    }
}
