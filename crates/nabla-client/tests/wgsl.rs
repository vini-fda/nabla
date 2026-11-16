//! The WGSL shaders parse and validate with naga, so a broken shader fails CI here rather
//! than at pipeline creation on a GPU.
#[test]
fn shaders_parse_and_validate() {
    let sources = [
        include_str!("../src/field.wgsl"),
        include_str!("../src/overlays.wgsl"),
        include_str!("../src/planets.wgsl"),
        include_str!("../src/ship3d.wgsl"),
        include_str!("../src/sprites.wgsl"),
        include_str!("../src/streamlines.wgsl"),
    ];
    for source in sources {
        let module = naga::front::wgsl::parse_str(source).expect("parses");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator.validate(&module).expect("validates");
    }
}
