# The AIS.ChatCraft collaboration relay. Build from the repository root:
#   docker build -f deploy/collab/Dockerfile -t aiscraft-collab-server .
FROM rust:1.95-slim-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p aiscraft-collab-server \
 && cp target/release/aiscraft-collab-server /aiscraft-collab-server

FROM debian:bookworm-slim
RUN useradd --system --uid 10001 --home /data collab && mkdir -p /data && chown collab /data
COPY --from=build /aiscraft-collab-server /usr/local/bin/aiscraft-collab-server
USER collab
ENV COLLAB_LISTEN=0.0.0.0:47910 COLLAB_DATA=/data
EXPOSE 47910
VOLUME /data
HEALTHCHECK --interval=30s --timeout=5s CMD bash -c 'exec 3<>/dev/tcp/127.0.0.1/47910 && printf "GET /healthz HTTP/1.0\r\n\r\n" >&3 && grep -q ok <&3'
ENTRYPOINT ["/usr/local/bin/aiscraft-collab-server"]
