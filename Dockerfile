FROM rust:1.80-slim AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build -p plc-daemon --release

FROM debian:bookworm-slim
RUN groupadd -r plc && useradd -r -g plc plc
WORKDIR /app
COPY --from=build /src/target/release/plc-daemon /usr/local/bin/plc-daemon
USER plc
ENTRYPOINT ["plc-daemon"]
