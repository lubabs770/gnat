fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).unwrap();
    let c = gnat_sim::Connectome::load(&path)?;
    println!(
        "neurons {} synapses {} types {:?}",
        c.neuron_count(),
        c.synapse_count(),
        c.type_names
    );
    for (i, n) in c.neurons.iter().enumerate() {
        let (t, w) = c.out_edges(i);
        println!(
            "  {} role {:?} type {} pos {:?} -> {:?} {:?}",
            n.root_id, n.role, c.type_names[n.cell_type as usize], n.pos, t, w
        );
    }
    println!("GF index: {:?}", c.by_type("GF"));
    Ok(())
}
