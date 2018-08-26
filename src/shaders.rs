use glium;
use glium_sdl2;

#[derive(Copy, Clone)]
pub struct Vertex {
    pub position: [f32; 3],
    pub color: [f32; 3],
}
implement_vertex!(Vertex, position, color);

pub fn load_shaders(display: &glium_sdl2::SDL2Facade) -> glium::Program {
    let program = program!(display,

            140 => {
                vertex: "
                    #version 140
                    uniform mat4 persp_matrix;
                    uniform mat4 view_matrix;
                    in vec3 position;
                    in vec3 color;
                    //in vec3 normal;
                    out vec3 v_position;
                    //out vec3 v_normal;
                    out vec3 v_color;
                    void main() {
                        v_position = position;
                        v_color = color;
                        //v_normal = normal;
                        gl_Position = persp_matrix * view_matrix * vec4(v_position, 1.0);
                    }
                ",

                fragment: "
                    #version 140
                    in vec3 v_color;
                    out vec4 f_color;
                    //const vec3 LIGHT = vec3(-0.2, 0.8, 0.1);
                    void main() {
                        //float lum = max(dot(normalize(v_normal), normalize(LIGHT)), 0.0);
                        //vec3 color = (0.3 + 0.7 * lum) * vec3(1.0, 1.0, 1.0);
                        //f_color = vec4(color, 1.0);
                        f_color = vec4(v_color, 1.0);
                    }
                ",
            },
    /*
        140 => {
            vertex: "
                #version 140
                uniform mat4 persp_matrix;
                uniform mat4 view_matrix;
                in vec3 position;
                in vec3 normal;
                out vec3 v_position;
                out vec3 v_normal;
                void main() {
                    v_position = position;
                    v_normal = normal;
                    gl_Position = persp_matrix * view_matrix * vec4(v_position * 0.005, 1.0);
                }
            ",

            fragment: "
                #version 140
                in vec3 v_normal;
                out vec4 f_color;
                const vec3 LIGHT = vec3(-0.2, 0.8, 0.1);
                void main() {
                    float lum = max(dot(normalize(v_normal), normalize(LIGHT)), 0.0);
                    vec3 color = (0.3 + 0.7 * lum) * vec3(1.0, 1.0, 1.0);
                    f_color = vec4(color, 1.0);
                }
            ",
        },
        110 => {
            vertex: "
                #version 110
                uniform mat4 persp_matrix;
                uniform mat4 view_matrix;
                attribute vec3 position;
                attribute vec3 normal;
                varying vec3 v_position;
                varying vec3 v_normal;
                void main() {
                    v_position = position;
                    v_normal = normal;
                    gl_Position = persp_matrix * view_matrix * vec4(v_position * 0.005, 1.0);
                }
            ",

            fragment: "
                #version 110
                varying vec3 v_normal;
                const vec3 LIGHT = vec3(-0.2, 0.8, 0.1);
                void main() {
                    float lum = max(dot(normalize(v_normal), normalize(LIGHT)), 0.0);
                    vec3 color = (0.3 + 0.7 * lum) * vec3(1.0, 1.0, 1.0);
                    gl_FragColor = vec4(color, 1.0);
                }
            ",
        },

        100 => {
            vertex: "
                #version 100
                uniform lowp mat4 persp_matrix;
                uniform lowp mat4 view_matrix;
                attribute lowp vec3 position;
                attribute lowp vec3 normal;
                varying lowp vec3 v_position;
                varying lowp vec3 v_normal;
                void main() {
                    v_position = position;
                    v_normal = normal;
                    gl_Position = persp_matrix * view_matrix * vec4(v_position * 0.005, 1.0);
                }
            ",

            fragment: "
                #version 100
                varying lowp vec3 v_normal;
                const lowp vec3 LIGHT = vec3(-0.2, 0.8, 0.1);
                void main() {
                    lowp float lum = max(dot(normalize(v_normal), normalize(LIGHT)), 0.0);
                    lowp vec3 color = (0.3 + 0.7 * lum) * vec3(1.0, 1.0, 1.0);
                    gl_FragColor = vec4(color, 1.0);
                }
            ",
        },*/
    ).unwrap();

    program
}
