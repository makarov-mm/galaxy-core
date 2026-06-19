use galaxy::math::{vec3, Mat4, Vec3};
use galaxy::octree::Octree;
use galaxy::sim::Sim;
use galaxy::galaxy::{make_disk, DiskParams};
use std::time::Instant;

fn rel_error(a: &[Vec3], b: &[Vec3]) -> (f32, f32) {
    let mut max = 0.0f32;
    let mut sum = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        let e = (*x - *y).length() / y.length().max(1e-6);
        max = max.max(e);
        sum += e;
    }
    (sum / a.len() as f32, max)
}

fn build_sim(n: usize, theta: f32) -> Sim {
    let p = DiskParams { n, ..Default::default() };
    let (pos, vel, mass) = make_disk(&p);
    Sim::new(pos, vel, mass, 0.002, theta, 0.08)
}

fn main() {
    println!("=== Barnes-Hut N-body core - verification ===\n");

    // 0) Camera math sanity: a point on the +Z axis in front of the camera must
    //    project near the screen centre with z in (-1, 1).
    println!("[0] Camera math");
    let view = Mat4::look_at(vec3(0.0, 0.0, 10.0), Vec3::ZERO, vec3(0.0, 1.0, 0.0));
    let proj = Mat4::perspective(60f32.to_radians(), 1.0, 0.1, 100.0);
    let vp = proj.mul(&view);
    let c = vp.transform_point(Vec3::ZERO);
    let off = vp.transform_point(vec3(2.0, 0.0, 0.0));
    println!("    origin -> ({:.3}, {:.3}, {:.3})  [expect ~0,0 and -1<z<1]", c.x, c.y, c.z);
    println!("    +x off -> ({:.3}, {:.3})         [expect x > 0]", off.x, off.y);

    // 1) Accuracy vs direct summation.
    println!("\n[1] Tree accuracy vs brute force (N = 4000)");
    let sim = build_sim(4000, 0.5);
    let direct = sim.brute_acc();
    for theta in [1.0f32, 0.7, 0.5, 0.3] {
        let tree = Octree::build(&sim.pos, &sim.mass, theta, sim.eps);
        let acc: Vec<Vec3> = (0..sim.len()).map(|i| tree.accel(i, sim.pos[i])).collect();
        let (mean, max) = rel_error(&acc, &direct);
        println!("    theta = {theta:<4}  mean rel.err = {:.4}%   max = {:.3}%", mean * 100.0, max * 100.0);
    }

    // 2) Energy conservation over time.
    println!("\n[2] Energy drift (N = 3000, 400 steps, theta = 0.5)");
    let mut sim = build_sim(3000, 0.5);
    let e0 = sim.total_energy();
    for _ in 0..400 {
        sim.step();
    }
    let e1 = sim.total_energy();
    println!("    E0 = {e0:.3}   E_final = {e1:.3}   drift = {:.4}%", (e1 - e0).abs() / e0.abs() * 100.0);

    // 3) Throughput.
    println!("\n[3] Throughput - ms per step (build tree + parallel force eval)");
    println!("    nproc = {}", std::thread::available_parallelism().map(|p| p.get()).unwrap_or(1));
    for n in [20_000usize, 50_000, 100_000, 200_000] {
        let mut sim = build_sim(n, 0.6);
        sim.step();
        let t = Instant::now();
        let iters = 5;
        for _ in 0..iters {
            sim.step();
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / iters as f64;
        println!("    N = {n:>7}   {ms:>7.1} ms/step   (~{:>4.0} fps)", 1000.0 / ms);
    }

    println!("\nCore OK.");
}
