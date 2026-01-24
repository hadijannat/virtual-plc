FROM rust:1.76-slim AS build
WORKDIR /src
COPY . .
RUN cargo build -p plc-daemon --release

FROM debian:bookworm-slim
WORKDIR /app
COPY --from=build /src/target/release/plc-daemon /usr/local/bin/plc-daemon
ENTRYPOINT ["plc-daemon"]
