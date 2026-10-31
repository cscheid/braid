use std::time::Instant;
use automerge::{Automerge, ReadDoc, ObjType, ObjId, Value, transaction::Transactable, ROOT};
use serde_json::Value as J;
fn put(tx:&mut automerge::transaction::Transaction, o:&ObjId, k:&str, v:&J){match v{J::Null=>{},J::String(x)=>{tx.put(o,k,x.as_str()).unwrap();},J::Number(n)=>{tx.put(o,k,n.as_i64().unwrap_or(0)).unwrap();},J::Bool(b)=>{tx.put(o,k,*b).unwrap();},J::Array(_)=>{tx.put(o,k,v.to_string()).unwrap();},J::Object(m)=>{let c=tx.put_object(o,k,ObjType::Map).unwrap();for(a,b) in m{put(tx,&c,a,b);}}}}
fn walk(d:&Automerge,o:&ObjId)->usize{let mut n=0;let keys:Vec<String>=d.keys(o).collect();for k in keys{match d.get(o,k.as_str()).unwrap(){Some((Value::Object(_),c))=>n+=walk(d,&c),Some((Value::Scalar(s),_))=>n+=s.to_str().map(|x|x.len()).unwrap_or(1),None=>{}}}n}
fn t<T>(name:&str, f:impl Fn()->T)->T{ let mut best=f64::MAX; let mut r=None;
  for _ in 0..3 { let s=Instant::now(); let x=f(); best=best.min(s.elapsed().as_secs_f64()*1e3); r=Some(x);} 
  println!("{name:42} {best:9.1} ms"); r.unwrap() }
fn main(){ let p=std::env::args().nth(1).unwrap(); let data=std::fs::read(&p).unwrap();
 let doc=t("Automerge::load (verify heads)",||Automerge::load(&data).unwrap());
 t("Automerge::load_unverified_heads",||Automerge::load_unverified_heads(&data).unwrap());
 t("load_incremental into empty (samod path)",||{let mut d=Automerge::new(); d.load_incremental(&data).unwrap(); d});
 let n=t("get_changes(&[]) (samod save_new_changes)",||doc.get_changes(&[]).len());
 let saved=t("doc.save()",||doc.save());
 let s=t("braid_core hydrate (all strands)",||braid_core::amdoc::hydrate(&doc).unwrap());
 t("hydrate_metadata",||braid_core::amdoc::hydrate_metadata(&doc).unwrap());
 println!("snapshot bytes={} save bytes={} changes={} ops={} strands={} heads={}",data.len(),saved.len(),n,doc.stats().num_ops,s.issues.len(),doc.get_heads().len());
 let ch=doc.get_changes(&[]); let mut actors=std::collections::HashSet::new(); for c in &ch {actors.insert(c.actor_id().clone());}
 println!("actors={}",actors.len());

 let (_,iss)=doc.get(ROOT,"issues").unwrap().unwrap();
 t("summary hydrate: id,title,status,prio,type,assignee",||{let mut n=0;let ks:Vec<String>=doc.keys(&iss).collect();for k in ks{let (_,o)=doc.get(&iss,k.as_str()).unwrap().unwrap();for f in ["id","title","status","priority","issue_type","assignee"]{if doc.get(&o,f).unwrap().is_some(){n+=1;}}}n});
 let mut sc=Automerge::new(); let root={let mut tx=sc.transaction();let r=tx.put_object(ROOT,"issues",ObjType::Map).unwrap();tx.commit();r};
 for (id,i) in &s.issues { let v=serde_json::to_value(i).unwrap(); let mut tx=sc.transaction(); put(&mut tx,&root,id,&v); tx.commit(); }
 let bytes=sc.save(); println!("--- same strands, all text as scalar strings: bytes={} ops={} changes={}",bytes.len(),sc.stats().num_ops,sc.get_changes(&[]).len());
 let d2=t("scalar: load",||Automerge::load(&bytes).unwrap());
 t("scalar: get_changes(&[])",||d2.get_changes(&[]).len());
 t("scalar: save",||d2.save());
 t("scalar: walk every field",||walk(&d2,&ROOT));
}
