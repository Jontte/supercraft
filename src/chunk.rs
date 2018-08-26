use database;
use glium;
use glium_sdl2;
use nalgebra;
use noise;
use std::sync::{Arc, RwLock};
use uuid;

use shaders::Vertex;

type Vector3i = nalgebra::Vector3<i32>;
type Vector3f = nalgebra::Vector3<f32>;

pub const CHUNK_SIDE: i32 = 16;
pub const CHUNK_CELLS: i32 = CHUNK_SIDE * CHUNK_SIDE * CHUNK_SIDE;

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
enum CellMaterial {
    VOID,
    GRASS,
    ROCK,
}

impl Default for CellMaterial {
    fn default() -> CellMaterial {
        CellMaterial::VOID
    }
}

#[derive(Clone, Copy)]
pub struct Face {
    brightness: f32,
    mat: CellMaterial,
}

impl Default for Face {
    fn default() -> Face {
        Face {
            brightness: 0.0,
            mat: CellMaterial::VOID,
        }
    }
}

#[derive(Clone, Copy, Serialize, Deserialize, Default)]
pub struct Cell {
    material: CellMaterial,

    #[serde(skip_serializing)]
    #[serde(skip_deserializing)]
    faces: [Option<Face>; 6],
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub position: Vector3i,
    pub cells: Vec<Cell>,

    #[serde(skip_serializing)]
    #[serde(skip_deserializing)]
    pub uuid: uuid::Uuid,
}
pub struct ChunkView {
    pub position: Vector3i,
    pub vertex_buffer: Option<glium::VertexBuffer<Vertex>>,
    pub index_buffer: Option<glium::IndexBuffer<u16>>,
}

impl ChunkView {
    pub fn new(position: Vector3i) -> ChunkView {
        ChunkView {
            position,
            vertex_buffer: None,
            index_buffer: None,
        }
    }
    pub fn update_mesh(&mut self, display: &glium_sdl2::SDL2Facade, mesh: (Vec<Vertex>, Vec<u16>)) {
        self.vertex_buffer = Some(glium::VertexBuffer::new(display, &mesh.0[..]).unwrap());
        self.index_buffer = Some(
            glium::IndexBuffer::new(
                display,
                glium::index::PrimitiveType::TrianglesList,
                &mesh.1[..],
            ).unwrap(),
        );
    }
}

impl Chunk {
    pub fn new(position: Vector3i) -> Chunk {
        Chunk {
            position,
            cells: vec![
                Cell {
                    material: CellMaterial::VOID,
                    ..Default::default()
                };
                CHUNK_CELLS as usize
            ],
            uuid: uuid::Uuid::nil(),
        }
    }
    pub fn generate(position: Vector3i, _seed: u64) -> Chunk {
        use noise::NoiseFn;
        let noise = noise::OpenSimplex::new();

        let mut chunk = Chunk::new(position);
        chunk.iter(|cell_offset, cell| {
            let global_pos = position * CHUNK_SIDE + cell_offset;

            let k = 0.05;
            let height = noise.get([
                (global_pos.x as f64) * k,
                (global_pos.y as f64) * k,
                (global_pos.z as f64) * k,
            ]);

            if height > 0.0 {
                cell.material = CellMaterial::ROCK;
            } else {
                cell.material = CellMaterial::VOID;
            }

            //println!("{}", val

            /*cell.material = match global_pos.z {
                _ if global_pos.z == height => CellMaterial::GRASS,
                _ if global_pos.z < height => CellMaterial::ROCK,
                _ => CellMaterial::VOID,
            };*/
        });
        chunk
    }
    pub fn iter<F: FnMut(Vector3i, &mut Cell)>(&mut self, mut func: F) {
        let mut counter = 0;
        for z in 0..CHUNK_SIDE {
            for y in 0..CHUNK_SIDE {
                for x in 0..CHUNK_SIDE {
                    func(Vector3i::new(x, y, z), &mut self.cells[counter]);
                    counter += 1;
                }
            }
        }
    }
    pub fn index(&self, position: Vector3i) -> Option<usize> {
        let position = position - self.position * CHUNK_SIDE;
        if position.x < 0 || position.x >= CHUNK_SIDE {
            return None;
        }
        if position.y < 0 || position.y >= CHUNK_SIDE {
            return None;
        }
        if position.z < 0 || position.z >= CHUNK_SIDE {
            return None;
        }
        Some((position.z * CHUNK_SIDE * CHUNK_SIDE + position.y * CHUNK_SIDE + position.x) as usize)
    }
    pub fn get(&self, position: Vector3i) -> Option<&Cell> {
        self.index(position).map(|i| &self.cells[i])
    }
    pub fn get_mut<'a>(&'a mut self, position: Vector3i) -> Option<&'a mut Cell> {
        self.index(position).map(move |i| &mut self.cells[i])
    }
    fn get_global(
        &mut self,
        neighbors: &mut [Option<&Arc<RwLock<Chunk>>>; 6],
        pos: Vector3i,
    ) -> Option<Cell> {
        let mut r: Option<Cell> = self.get(pos).map(|x| x.clone());
        for n in neighbors.iter_mut() {
            if r.is_none() {
                r = n
                    .as_mut()
                    .and_then(|n| n.read().unwrap().get(pos).map(|x| x.clone()));
            }
        }
        r
    }
    pub fn update_polymesh(
        &mut self,
        mut neighbors: [Option<&Arc<RwLock<Chunk>>>; 6],
    ) -> (Vec<Vertex>, Vec<u16>) {
        let mut vertices: Vec<Vertex> = vec![];
        let mut indices: Vec<u16> = vec![];
        {
            for z in 0..CHUNK_SIDE {
                for y in 0..CHUNK_SIDE {
                    for x in 0..CHUNK_SIDE {
                        let global_position: Vector3i =
                            self.position * CHUNK_SIDE + Vector3i::new(x, y, z);

                        {
                            // Solid cells don't need to render anything
                            let mut cell = self.get(global_position).unwrap();
                            match cell.material {
                                CellMaterial::VOID => {}
                                _ => continue,
                            }
                        }

                        // check neighbors
                        let n_cells = [
                            self.get_global(
                                &mut neighbors,
                                global_position + Vector3i::new(-1, 0, 0),
                            ),
                            self.get_global(
                                &mut neighbors,
                                global_position + Vector3i::new(1, 0, 0),
                            ),
                            self.get_global(
                                &mut neighbors,
                                global_position + Vector3i::new(0, -1, 0),
                            ),
                            self.get_global(
                                &mut neighbors,
                                global_position + Vector3i::new(0, 1, 0),
                            ),
                            self.get_global(
                                &mut neighbors,
                                global_position + Vector3i::new(0, 0, -1),
                            ),
                            self.get_global(
                                &mut neighbors,
                                global_position + Vector3i::new(0, 0, 1),
                            ),
                        ];

                        let mut cell = self.get_mut(global_position).unwrap();

                        const FACES_COORDS: [[[f32; 3]; 4]; 6] = [
                            [
                                [0.0, 0.0, 0.0],
                                [0.0, 1.0, 0.0],
                                [0.0, 0.0, 1.0],
                                [0.0, 1.0, 1.0],
                            ],
                            [
                                [1.0, 0.0, 0.0],
                                [1.0, 0.0, 1.0],
                                [1.0, 1.0, 0.0],
                                [1.0, 1.0, 1.0],
                            ],
                            [
                                [0.0, 0.0, 0.0],
                                [1.0, 0.0, 0.0],
                                [0.0, 0.0, 1.0],
                                [1.0, 0.0, 1.0],
                            ],
                            [
                                [0.0, 1.0, 0.0],
                                [0.0, 1.0, 1.0],
                                [1.0, 1.0, 0.0],
                                [1.0, 1.0, 1.0],
                            ],
                            [
                                [0.0, 0.0, 0.0],
                                [0.0, 1.0, 0.0],
                                [1.0, 0.0, 0.0],
                                [1.0, 1.0, 0.0],
                            ],
                            [
                                [0.0, 0.0, 1.0],
                                [1.0, 0.0, 1.0],
                                [0.0, 1.0, 1.0],
                                [1.0, 1.0, 1.0],
                            ],
                        ];

                        for i in 0..6 {
                            match n_cells[i].map(|x| x.material).unwrap_or(CellMaterial::VOID) {
                                CellMaterial::VOID => {
                                    // skip face
                                    cell.faces[i] = None;
                                }
                                mat => {
                                    // generate face
                                    cell.faces[i] = Some(Face {
                                        brightness: 1.0,
                                        mat,
                                    });
                                }
                            }
                        }
                        for i in 0..6 {
                            if cell.faces[i].is_none() {
                                continue;
                            }
                            let index_offset = vertices.len() as u16;
                            for index in [0, 1, 2, 2, 1, 3].iter() {
                                indices.push(index + index_offset);
                            }
                            for v in FACES_COORDS[i].iter() {
                                let mut ao_score = 0;
                                for a in 0..6 {
                                    if a == i || cell.faces[a].is_none() {
                                        continue;
                                    }
                                    if FACES_COORDS[a]
                                        .iter()
                                        .filter(|v2| *v2 == v)
                                        .nth(0)
                                        .is_some()
                                    {
                                        ao_score += 1;
                                    }
                                }

                                let position = [
                                    global_position.x as f32 + v[0],
                                    global_position.y as f32 + v[1],
                                    global_position.z as f32 + v[2],
                                ];

                                let mut color = match cell.faces[i].unwrap().mat {
                                    CellMaterial::VOID => Vector3f::new(0.0, 0.0, 0.0),
                                    CellMaterial::GRASS => Vector3f::new(0.0, 1.0, 0.0),
                                    CellMaterial::ROCK => Vector3f::new(0.5, 0.5, 0.5),
                                };

                                if ao_score == 1 {
                                    color *= 0.5;
                                }
                                if ao_score == 2 {
                                    color *= 0.3;
                                }

                                vertices.push(Vertex {
                                    position: position,
                                    color: *color.as_ref(),
                                })
                            }
                        }
                    }
                }
            }
            (vertices, indices)
        }
    }
}

impl database::DBObject for Chunk{
    fn db_key() -> &'static [u8; 3] {
        b"CHK"
    }
    fn get_uuid(&self) -> uuid::Uuid {
        self.uuid
    }
    fn get_references(&self) -> Vec<uuid::Uuid> {
        Vec::new()
    }
}
