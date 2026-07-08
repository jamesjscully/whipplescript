# Class-A executor container (compute plane P8): the `whip executor` sidecar
# that runs sha-pinned scripts for the DO runtime (whip-executor/1 wire).
# Build context = this directory; `npm run build:executor` drops the release
# `whip` binary here first. The image digest is the environment hash the
# delta-kernel cache keys on (the image-digest wiring box).
# trixie's glibc (2.41) covers binaries built on current hosts (2.39); the
# release pipeline can later pin a musl-static build for a smaller image.
FROM debian:trixie-slim
COPY whip /usr/local/bin/whip
EXPOSE 8080
ENTRYPOINT ["whip", "executor", "--bind", "0.0.0.0:8080"]
