fn main() {
    tonic_prost_build::compile_protos("../recipe-scraper-py/src/proto/recipe.proto").unwrap();
}
