fn main() {
    embuild::espidf::sysenv::output();

    // The Makefile rewrites the partition path into target/sdkconfig.partitions;
    // still re-run when the table itself changes.
    println!("cargo:rerun-if-changed=partitions.csv");
    println!("cargo:rerun-if-changed=sdkconfig.defaults");
    println!("cargo:rerun-if-changed=components/hub75_shim/include/hub75_shim.h");
    println!("cargo:rerun-if-changed=components/hub75_shim/hub75_shim.cpp");
}
