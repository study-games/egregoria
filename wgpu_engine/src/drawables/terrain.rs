use crate::{
    bg_layout_litmesh, pbuffer::PBuffer, CompiledModule, Drawable, FrameContext, GfxContext,
    IndexType, Material, Mesh, MeshBuilder, MeshVertex, MetallicRoughness, PipelineBuilder,
    RenderParams, TerrainVertex, Texture, Uniform, TL,
};
use common::FastMap;
use geom::{
    vec2, vec3, Camera, InfiniteFrustrum, Intersect3, LinearColor, Matrix4, Polygon, Vec2, AABB3,
};
use std::ops::Sub;
use std::sync::Arc;
use wgpu::{
    BufferUsages, Extent3d, FilterMode, ImageCopyTexture, ImageDataLayout, IndexFormat, Origin3d,
    RenderPass, RenderPipeline, TextureFormat, TextureUsages, VertexAttribute, VertexBufferLayout,
};

const LOD: usize = 4;

pub struct TerrainChunk {
    pub dirt_id: u32,
}

pub struct TerrainRender<const CSIZE: usize, const CRESOLUTION: usize> {
    pub dirt_id: u32,
    dirt_ids: FastMap<(u32, u32), u32>,
    terrain_tex: Arc<Texture>,
    #[allow(unused)]
    grass_tex: Arc<Texture>, // kept alive
    borders: Arc<Vec<Mesh>>,
    vertices: [PBuffer; LOD],
    indices: [(PBuffer, u32); LOD],
    instances: [(PBuffer, u32); LOD],
    bg: Arc<wgpu::BindGroup>,
    border_col: LinearColor,
    cell_size: f32,
    w: u32,
    h: u32,
}

pub struct TerrainPrepared {
    terrainbg: Arc<wgpu::BindGroup>,
    vertices: [PBuffer; LOD],
    indices: [(PBuffer, u32); LOD],
    instances: [(PBuffer, u32); LOD],
}

impl<const CSIZE: usize, const CRESOLUTION: usize> TerrainRender<CSIZE, CRESOLUTION> {
    pub fn new(
        gfx: &mut GfxContext,
        w: u32,
        h: u32,
        col: LinearColor,
        grass: Arc<Texture>,
    ) -> Self {
        let (indices, vertices) = Self::generate_indices_mesh(gfx);
        let mut tex = Texture::create_fbo(
            &gfx.device,
            (w * CRESOLUTION as u32 + 2, h * CRESOLUTION as u32 + 2),
            TextureFormat::R32Float,
            TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING,
            None,
        );
        tex.sampler = gfx.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("texture sampler"),
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        defer!(log::info!("finished init of terrain render"));
        Self {
            bg: Arc::new(Texture::multi_bindgroup(
                &[&tex, &grass],
                &gfx.device,
                &Texture::bindgroup_layout(&gfx.device, [TL::NonfilterableFloat, TL::Float]),
            )),
            dirt_ids: Default::default(),
            terrain_tex: Arc::new(tex),
            grass_tex: grass,
            borders: Arc::new(vec![]),
            indices,
            vertices,
            dirt_id: 0,
            cell_size: CSIZE as f32 / CRESOLUTION as f32,
            w,
            h,
            instances: collect_arrlod((0..LOD).map(|_| (PBuffer::new(BufferUsages::VERTEX), 0))),
            border_col: col,
        }
    }

    pub fn reset(&mut self) {
        self.dirt_id = 0;
        self.dirt_ids.clear();
    }

    pub fn update_chunk(
        &mut self,
        gfx: &mut GfxContext,
        dirtid: u32,
        cell: (u32, u32),
        chunk: &[[f32; CRESOLUTION]; CRESOLUTION],
    ) -> bool {
        if self
            .dirt_ids
            .get(&cell)
            .map(|x| *x == dirtid)
            .unwrap_or_default()
        {
            return false;
        }

        let mut contents = Vec::with_capacity(CRESOLUTION * CRESOLUTION);

        let extrax = cell.0 + 1 == self.w;
        let extray = cell.1 + 1 == self.h;

        let w = CRESOLUTION as u32 + 2 * extrax as u32;
        let h = CRESOLUTION as u32 + 2 * extray as u32;

        for y in chunk
            .iter()
            .chain(extray.then(|| &chunk[CRESOLUTION - 1]).into_iter())
            .chain(extray.then(|| &chunk[CRESOLUTION - 1]).into_iter())
        {
            for x in y {
                contents.extend(x.to_le_bytes());
            }
            if extrax {
                contents.extend(y[y.len() - 1].to_le_bytes());
                contents.extend(y[y.len() - 1].to_le_bytes());
            }

            if w * 4 < wgpu::COPY_BYTES_PER_ROW_ALIGNMENT {
                contents.resize(
                    contents.len() + wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize - w as usize * 4,
                    0,
                );
            }
        }

        gfx.queue.write_texture(
            ImageCopyTexture {
                texture: &self.terrain_tex.texture,
                mip_level: 0,
                origin: Origin3d {
                    x: cell.0 * CRESOLUTION as u32,
                    y: cell.1 * CRESOLUTION as u32,
                    z: 0,
                },
                aspect: Default::default(),
            },
            &contents,
            ImageDataLayout {
                offset: 0,
                bytes_per_row: Some((w * 4).max(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)),
                rows_per_image: Some(h),
            },
            Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );

        self.dirt_ids.insert(cell, dirtid);
        true
    }

    #[profiling::function]
    pub fn draw_terrain(
        &mut self,
        cam: &Camera,
        frustrum: &InfiniteFrustrum,
        fctx: &mut FrameContext<'_>,
    ) {
        for b in self.borders.iter() {
            fctx.objs.push(Box::new(b.clone()));
        }

        let eye = cam.eye();

        let mut instances = vec![Vec::<TerrainInstance>::new(); LOD];
        for y in 0..self.h {
            for x in 0..self.w {
                let p = vec2(x as f32, y as f32) * CSIZE as f32;

                if !frustrum.intersects(&AABB3::centered(
                    (p + Vec2::splat(CSIZE as f32 * 0.5)).z(0.0),
                    vec3(CSIZE as f32, CSIZE as f32, 1000.0),
                )) {
                    continue;
                }

                let lod = eye.distance(p.z0()).log2().sub(10.0).max(0.0) as usize;
                let lod = lod.min(LOD - 1);

                instances[lod].push(TerrainInstance { offset: p })
            }
        }

        for (lod, instance) in instances.into_iter().enumerate() {
            self.instances[lod].1 = instance.len() as u32;
            self.instances[lod]
                .0
                .write(fctx.gfx, bytemuck::cast_slice(&instance));
        }

        fctx.objs.push(Box::new(TerrainPrepared {
            terrainbg: self.bg.clone(),
            vertices: self.vertices.clone(),
            indices: self.indices.clone(),
            instances: self.instances.clone(),
        }));
    }

    pub fn update_borders(&mut self, gfx: &mut GfxContext, height: &dyn Fn(Vec2) -> Option<f32>) {
        let minx = unwrap_ret!(self.dirt_ids.keys().map(|x| x.0).min());
        let maxx = unwrap_ret!(self.dirt_ids.keys().map(|x| x.0).max()) + 1;
        let miny = unwrap_ret!(self.dirt_ids.keys().map(|x| x.1).min());
        let maxy = unwrap_ret!(self.dirt_ids.keys().map(|x| x.1).max()) + 1;
        let cell_size = self.cell_size;
        let mut mk_bord = |start, end, c, is_x, rev| {
            let c = c as f32 * CSIZE as f32;
            let flip = move |v: Vec2| {
                if is_x {
                    v
                } else {
                    vec2(v.y, v.x)
                }
            };

            let mut poly = Polygon(vec![]);
            poly.0.push(vec2(start as f32 * CSIZE as f32, -3000.0));
            for along in start * CRESOLUTION as u32..=end * CRESOLUTION as u32 {
                let along = along as f32 * cell_size;
                let p = flip(vec2(along, c));
                let height = unwrap_cont!(height(p - (p - Vec2::splat(3.0)).sign() * 1.0));
                poly.0.push(vec2(along, height + 1.5));
            }
            poly.0.push(vec2(end as f32 * CSIZE as f32, -3000.0));

            poly.simplify();

            let mut indices = vec![];
            crate::earcut::earcut(&poly.0, |mut a, b, mut c| {
                if rev {
                    std::mem::swap(&mut a, &mut c);
                }
                indices.push(a as IndexType);
                indices.push(b as IndexType);
                indices.push(c as IndexType);
            });
            let mat = gfx.register_material(Material::new(
                gfx,
                gfx.palette(),
                MetallicRoughness {
                    metallic: 0.0,
                    roughness: 1.0,
                    tex: None,
                },
                None,
            ));
            let mut mb = MeshBuilder::new(mat);
            mb.indices = indices;
            mb.vertices = poly
                .0
                .into_iter()
                .map(|p| MeshVertex {
                    position: if is_x {
                        vec3(p.x, c, p.y)
                    } else {
                        vec3(c, p.x, p.y)
                    }
                    .into(),
                    normal: if rev ^ !is_x { 1.0 } else { -1.0 }
                        * vec3(!is_x as i32 as f32, is_x as i32 as f32, 0.0),
                    uv: [0.0, 0.0],
                    color: self.border_col.into(),
                    tangent: [0.0; 4],
                })
                .collect();
            mb.build(gfx)
        };

        let borders = Arc::get_mut(&mut self.borders).unwrap();
        borders.clear();
        borders.extend(mk_bord(minx, maxx, miny, true, false));
        borders.extend(mk_bord(minx, maxx, maxy, true, true));
        borders.extend(mk_bord(miny, maxy, minx, false, true));
        borders.extend(mk_bord(miny, maxy, maxx, false, false));
    }

    fn generate_indices_mesh(gfx: &GfxContext) -> ([(PBuffer, u32); LOD], [PBuffer; LOD]) {
        let mut indlod = vec![];
        let mut vertlod = vec![];
        let cell_size = (CSIZE / CRESOLUTION) as f32;
        for lod in 0..LOD {
            let scale = 1 << lod;
            let resolution = CRESOLUTION / scale;

            let mut indices: Vec<IndexType> = Vec::with_capacity(6 * resolution * resolution);
            let mut vertices: Vec<TerrainVertex> =
                Vec::with_capacity((resolution + 1) * (resolution + 1));

            let resolution = resolution as IndexType;
            let w = resolution + 1;
            for y in 0..=resolution {
                for x in 0..=resolution {
                    let pos = vec2(x as f32, y as f32) * cell_size * scale as f32;
                    vertices.push(TerrainVertex {
                        position: [pos.x, pos.y],
                    });

                    if x < resolution && y < resolution {
                        let idx = y * w + x;
                        indices.push(idx);
                        indices.push(idx + 1);
                        indices.push(idx + w + 1);

                        indices.push(idx);
                        indices.push(idx + w + 1);
                        indices.push(idx + w);
                    }
                }
            }

            let l = indices.len();

            let mut buf = PBuffer::new(BufferUsages::INDEX);
            buf.write(gfx, bytemuck::cast_slice(&indices));
            indlod.push((buf, l as u32));

            let mut buf = PBuffer::new(BufferUsages::VERTEX);
            buf.write(gfx, bytemuck::cast_slice(&vertices));
            vertlod.push(buf);
        }
        (collect_arrlod(indlod), collect_arrlod(vertlod))
    }
}

#[derive(Hash)]
struct TerrainPipeline {
    depth: bool,
    smap: bool,
}

#[derive(Copy, Clone)]
#[repr(C)]
pub struct TerrainInstance {
    pub offset: Vec2,
}

u8slice_impl!(TerrainInstance);

const ATTRS: &[VertexAttribute] = &wgpu::vertex_attr_array![1 => Float32x2];

impl TerrainInstance {
    fn desc() -> VertexBufferLayout<'static> {
        VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: ATTRS,
        }
    }
}

impl TerrainPrepared {
    fn set_buffers<'a>(&'a self, rp: &mut RenderPass<'a>) {
        for lod in 0..LOD {
            let (ind, n_indices) = &self.indices[lod];
            let vertices = &self.vertices[lod];
            let (instances, n_instances) = &self.instances[lod];

            if *n_instances == 0 {
                continue;
            }

            rp.set_vertex_buffer(0, vertices.slice().unwrap());
            rp.set_vertex_buffer(1, instances.slice().unwrap());
            rp.set_index_buffer(ind.slice().unwrap(), IndexFormat::Uint32);
            rp.draw_indexed(0..*n_indices, 0, 0..*n_instances);
        }
    }
}

impl PipelineBuilder for TerrainPipeline {
    fn build(
        &self,
        gfx: &GfxContext,
        mut mk_module: impl FnMut(&str) -> CompiledModule,
    ) -> RenderPipeline {
        let terrainlayout =
            Texture::bindgroup_layout(&gfx.device, [TL::NonfilterableFloat, TL::Float]);
        let vert = &mk_module("terrain.vert");

        if !self.depth {
            let frag = &mk_module("terrain.frag");

            return gfx.color_pipeline(
                "terrain",
                &[
                    &gfx.projection.layout,
                    &Uniform::<RenderParams>::bindgroup_layout(&gfx.device),
                    &terrainlayout,
                    &bg_layout_litmesh(&gfx.device),
                ],
                &[TerrainVertex::desc(), TerrainInstance::desc()],
                vert,
                frag,
            );
        }

        gfx.depth_pipeline_bglayout(
            &[TerrainVertex::desc(), TerrainInstance::desc()],
            vert,
            None,
            self.smap,
            &[
                &gfx.projection.layout,
                &gfx.render_params.layout,
                &terrainlayout,
            ],
        )
    }
}

impl Drawable for TerrainPrepared {
    fn draw<'a>(&'a self, gfx: &'a GfxContext, rp: &mut RenderPass<'a>) {
        let pipeline = gfx.get_pipeline(TerrainPipeline {
            depth: false,
            smap: false,
        });

        rp.set_pipeline(pipeline);
        rp.set_bind_group(0, &gfx.projection.bindgroup, &[]);
        rp.set_bind_group(1, &gfx.render_params.bindgroup, &[]);
        rp.set_bind_group(2, &self.terrainbg, &[]);
        rp.set_bind_group(3, &gfx.simplelit_bg, &[]);

        self.set_buffers(rp);
    }

    fn draw_depth<'a>(
        &'a self,
        gfx: &'a GfxContext,
        rp: &mut RenderPass<'a>,
        shadow_cascade: Option<&Matrix4>,
        proj: &'a wgpu::BindGroup,
    ) {
        if shadow_cascade.is_some() {
            return;
        }
        rp.set_pipeline(gfx.get_pipeline(TerrainPipeline {
            depth: true,
            smap: false,
        }));
        rp.set_bind_group(0, proj, &[]);
        rp.set_bind_group(1, &gfx.render_params.bindgroup, &[]);
        rp.set_bind_group(2, &self.terrainbg, &[]);

        self.set_buffers(rp);
    }
}

fn collect_arrlod<T>(x: impl IntoIterator<Item = T>) -> [T; LOD] {
    let mut iter = x.into_iter();
    [(); LOD].map(move |_| iter.next().expect("iterator too short"))
}
