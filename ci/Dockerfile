FROM ubuntu:latest as fuse-zstd-test

ARG DEBIAN_FRONTEND=noninteractive

# install basic packages
RUN \
	apt-get update && \
	apt-get -y upgrade && \
	apt-get -y install --no-install-recommends \
		ca-certificates git cmake make pkg-config gcc g++ openssh-client \
		curl locales libfuse3-dev fuse3 \
		&& \
	rm -rf /var/lib/apt/lists/*

# generate locales
RUN \
	echo "en_US.UTF-8 UTF-8" > /etc/locale.gen && \
	locale-gen
ENV LC_ALL=en_US.UTF-8
ENV LANG=en_US.UTF-8

# install rust
RUN \
	curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="${PATH}:/root/.cargo/bin"

# install rust packages
RUN \
	cargo install cargo-deb

# Allow other users to mount fuse
RUN \
	echo "user_allow_other" >> /etc/fuse.conf
