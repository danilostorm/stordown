FROM rust:1-bookworm AS builder
WORKDIR /src
COPY . .
RUN cargo build --release -p stordown-relay

FROM debian:bookworm-slim
RUN useradd -r -u 10001 -m stordown && mkdir -p /data && chown -R stordown:stordown /data
COPY --from=builder /src/target/release/stordown-relay /usr/local/bin/stordown-relay
USER stordown
ENV STORDOWN_RELAY_BIND=0.0.0.0:17834
ENV STORDOWN_RELAY_DATA=/data
EXPOSE 17834
VOLUME ["/data"]
ENTRYPOINT ["stordown-relay"]
