use rusty_tiles::{glb_write::TilePrimitive,hlod::two_sided_error};
fn main() {
 let parent=TilePrimitive{positions:vec![[0.,0.,0.],[1.,0.,0.],[0.,1.,0.]],indices:vec![0,1,2],..Default::default()};
 let mut child=TilePrimitive::default();
 for i in 0..3000 {
  let z=if i==1 {100.} else {0.};
  let base=child.positions.len() as u32;
  child.positions.extend([[0.,0.,0.],[1.,0.,z],[0.,1.,z]]);
  child.indices.extend([base,base+1,base+2]);
 }
 let actual=two_sided_error(&[parent],&[child]);
 println!("{{\"child_triangles\":3000,\"estimated_error_metres\":{actual},\"independent_max_child_vertex_distance_metres\":100}}");
}
