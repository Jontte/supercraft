use glium;
use glium_sdl2;
use uuid;
use database;
use noise;

use shaders::Vertex;

pub const CHUNK_SIDE: i32 = 8;
pub const CHUNK_CELLS: i32 = CHUNK_SIDE * CHUNK_SIDE * CHUNK_SIDE;

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
enum CellMaterial {
    VOID,
    GRASS,
    ROCK
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct Cell {
    material: CellMaterial
}

#[derive(Serialize, Deserialize)]
pub struct Chunk {
    pub position: (i32, i32, i32),
    pub cells: Vec<Cell>,

    #[serde(skip_serializing)]
    #[serde(skip_deserializing)]
    ambient_occlusion: f32,

    #[serde(skip_serializing)]
    #[serde(skip_deserializing)]
    pub vertex_buffer: Option<glium::VertexBuffer<Vertex>>,

    #[serde(skip_serializing)]
    #[serde(skip_deserializing)]
    pub index_buffer: Option<glium::IndexBuffer<u16>>,

    #[serde(skip_serializing)]
    #[serde(skip_deserializing)]
    pub remove: bool, // unload from memory when possible

    #[serde(skip_serializing)]
    #[serde(skip_deserializing)]
    pub uuid: uuid::Uuid
}

impl Chunk {
    pub fn new(position: (i32, i32, i32), uuid: uuid::Uuid) -> Chunk {
        Chunk {
            position,
            cells: vec![Cell{material: CellMaterial::VOID}; CHUNK_CELLS as usize],
            vertex_buffer: None,
            index_buffer: None,
            remove: false,
            uuid,
            ambient_occlusion: 0.0
        }
    }
    pub fn generate(position: (i32, i32, i32), uuid: uuid::Uuid, seed: u64) -> Chunk {

        use noise::NoiseFn;
        let noise = noise::OpenSimplex::new();

        let mut chunk = Chunk::new(position, uuid);
        chunk.iter(|(x, y, z), cell| {
            let x = position.0 * CHUNK_SIDE + x;
            let y = position.1 * CHUNK_SIDE + y;
            let z = position.2 * CHUNK_SIDE + z;

            let k = 0.05;
            let height = noise.get([
                (x as f64) * k,
                (y as f64) * k,
                0.0f64]) * 30.0;


            let height = height as i32 - 10;

            //println!("{}", val

            cell.material = match z {
                 _ if z == height => { CellMaterial::GRASS },
                 _ if z < height => { CellMaterial::ROCK },
                 _ => { CellMaterial::VOID }
             };
        });
        chunk
    }
    pub fn iter<F: FnMut((i32, i32, i32), &mut Cell)>(&mut self, mut func: F) {
        let mut counter = 0;
        for z in 0..CHUNK_SIDE {
            for y in 0..CHUNK_SIDE {
                for x in 0..CHUNK_SIDE {
                    func((x, y, z), &mut self.cells[counter]);
                    counter += 1;
                }
            }
        }
    }
    pub fn update_buffers(&mut self, display: &glium_sdl2::SDL2Facade) {

        let mut vertices: Vec<Vertex> = vec![];
        let mut indices: Vec<u16> = vec![];

        let mut counter = 0;
        for z in 0..CHUNK_SIDE {
            for y in 0..CHUNK_SIDE {
                for x in 0..CHUNK_SIDE {
                    let cell = self.cells[counter];

                    if cell.material != CellMaterial::VOID {

                        let cube_vertices = [
                            [-1.0, -1.0,  1.0],
                            [1.0, -1.0,  1.0],
                            [1.0,  1.0,  1.0],
                            [-1.0,  1.0,  1.0],
                            [-1.0, -1.0, -1.0],
                            [1.0, -1.0, -1.0],
                            [1.0,  1.0, -1.0],
                            [-1.0,  1.0, -1.0]
                        ];

                        let cube_indices = [
                    		0, 1, 2,
                    		2, 3, 0,
                    		1, 5, 6,
                    		6, 2, 1,
                    		7, 6, 5,
                    		5, 4, 7,
                    		4, 0, 3,
                    		3, 7, 4,
                    		4, 5, 1,
                    		1, 0, 4,
                    		3, 2, 6,
                    		6, 7, 3,
                        ];

                        let index_offset = vertices.len() as u16;
                        for index in cube_indices.iter() {
                            indices.push(index + index_offset);
                        }

                        let color = match cell.material {
                            CellMaterial::VOID => { [0.0,0.0,0.0]},
                            CellMaterial::GRASS => { [0.0,1.0,0.0]},
                            CellMaterial::ROCK => { [0.5,0.5,0.5]}
                        };

                        for vertex in cube_vertices.iter() {
                            let vertex = [
                                vertex[0]/2.0+0.5 + (x + self.position.0 * CHUNK_SIDE) as f32,
                                vertex[1]/2.0+0.5 + (y + self.position.1 * CHUNK_SIDE) as f32,
                                vertex[2]/2.0+0.5 + (z + self.position.2 * CHUNK_SIDE) as f32
                            ];
                            vertices.push(Vertex { position : vertex, color: color});
                        }
                    }
                    counter += 1;
                }
            }
        }

        self.vertex_buffer = Some(glium::VertexBuffer::new(display, &vertices[..]).unwrap());
        self.index_buffer  = Some(glium::IndexBuffer::new(display, glium::index::PrimitiveType::TrianglesList, &indices[..]).unwrap());
    }
}

impl database::DBObject for Chunk {
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
