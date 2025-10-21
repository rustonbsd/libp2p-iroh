FROM rust:latest as chef
RUN cargo install cargo-chef
WORKDIR /app

FROM chef as planner
COPY Cargo.toml Cargo.lock ./
COPY README.md ./
COPY src ./src
COPY examples ./examples
RUN cargo chef prepare --recipe-path recipe.json

FROM chef as builder
COPY --from=planner /app/recipe.json recipe.json

RUN cargo chef cook --release --recipe-path recipe.json

COPY Cargo.toml Cargo.lock ./
COPY README.md ./
COPY src ./src
COPY examples ./examples

RUN cargo build --release --example e2e_test

RUN cargo test --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/examples/e2e_test /usr/local/bin/e2e_test

CMD ["e2e_test"]



