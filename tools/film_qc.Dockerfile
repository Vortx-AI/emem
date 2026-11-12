FROM debian:trixie-slim
RUN apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends ffmpeg python3 >/dev/null && rm -rf /var/lib/apt/lists/*
