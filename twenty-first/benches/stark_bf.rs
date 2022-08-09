use brainfuck::vm::sample_programs;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use twenty_first::shared_math::b_field_element::BFieldElement;
use twenty_first::shared_math::stark::brainfuck;
use twenty_first::shared_math::stark::brainfuck::memory_table::MemoryTable;
use twenty_first::shared_math::stark::brainfuck::stark::Stark;
use twenty_first::shared_math::stark::brainfuck::vm::BaseMatrices;
use twenty_first::timing_reporter::TimingReporter;

fn compile_simulate_prove_verify(program_code: &str, input: &[BFieldElement]) {
    let mut timer = TimingReporter::start();

    let program = brainfuck::vm::compile(program_code).unwrap();

    timer.elapsed("compile");

    let (trace_length, input_symbols, output_symbols) =
        brainfuck::vm::run(&program, input.to_vec()).unwrap();
    timer.elapsed("run");
    println!("run done");

    let base_matrices: BaseMatrices = brainfuck::vm::simulate(&program, &input_symbols).unwrap();
    let mt = MemoryTable::derive_matrix(
        base_matrices
            .processor_matrix
            .iter()
            .map(|reg| Into::<Vec<BFieldElement>>::into(reg.to_owned()))
            .collect(),
    );
    timer.elapsed("simulate");

    // Standard high parameters
    let log_expansion_factor = 4;
    let security_level = 160;

    let mut stark = Stark::new(
        trace_length,
        program_code.to_string(),
        input_symbols,
        output_symbols,
        log_expansion_factor,
        security_level,
        mt.len(),
    );
    timer.elapsed("new");

    let mut proof_stream = stark.prove(base_matrices, None).unwrap();
    timer.elapsed("prove");

    let verifier_verdict = stark.verify(&mut proof_stream);
    timer.elapsed("verify");
    let report = timer.finish();

    match verifier_verdict {
        Ok(_) => (),
        Err(err) => panic!("error in STARK verifier: {}", err),
    };
    println!("{}", report);
}

fn stark_bf(c: &mut Criterion) {
    let mut group = c.benchmark_group("stark_bf");

    group.sample_size(10);

    // Last time I checked this produces a FRI domain length of 2^14
    let two_by_two_then_output_id = BenchmarkId::new("TWO_BY_TWO_THEN_OUTPUT", 97);
    let tbtto_input = [97].map(BFieldElement::new).to_vec();
    group.bench_with_input(
        two_by_two_then_output_id,
        &tbtto_input,
        |bencher, input_symbols| {
            bencher.iter(|| {
                compile_simulate_prove_verify(
                    sample_programs::TWO_BY_TWO_THEN_OUTPUT,
                    input_symbols,
                )
            });
        },
    );

    // Last time I checked this produces a FRI domain length of 2^18
    let hello_world_id = BenchmarkId::new("HELLO_WORLD", "");
    group.bench_function(hello_world_id, |bencher| {
        bencher.iter(|| compile_simulate_prove_verify(sample_programs::HELLO_WORLD, &[]));
    });

    // Last time I checked this produces a FRI domain length of 2^21
    // let the_raven_id = BenchmarkId::new("THE_RAVEN", "");
    // group.bench_function(the_raven_id, |bencher| {
    //     bencher.iter(|| compile_simulate_prove_verify(sample_programs::THE_RAVEN, &[]));
    // });\

    // Last time I checked this produces a FRI domain length of 2^22
    // let the_raven_id = BenchmarkId::new("FIRST_FOUR_VERSES_OF_THE_RAVEN", "");
    // group.bench_function(the_raven_id, |bencher| {
    //     bencher.iter(|| {
    //         compile_simulate_prove_verify(sample_programs::FIRST_FOUR_VERSES_OF_THE_RAVEN, &[])
    //     });
    // });

    // Last time I checked this produces a FRI domain length of 2^23
    // let the_raven_id = BenchmarkId::new("FIRST_EIGHT_VERSES_OF_THE_RAVEN", "");
    // group.bench_function(the_raven_id, |bencher| {
    //     bencher.iter(|| {
    //         compile_simulate_prove_verify(sample_programs::FIRST_EIGHT_VERSES_OF_THE_RAVEN, &[])
    //     });
    // });

    // The following benchmark will crash unless you have 128GiB RAM. FRI domain length: 2^24
    // let the_whole_raven_id = BenchmarkId::new("THE_WHOLE_RAVEN", 0);
    // group.bench_function(the_whole_raven_id, |bencher| {
    //     bencher.iter(|| compile_simulate_prove_verify(sample_programs::THE_WHOLE_RAVEN, &[]));
    // });

    group.finish();
}

criterion_group!(benches, stark_bf);
criterion_main!(benches);
