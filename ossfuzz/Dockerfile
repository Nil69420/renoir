FROM gcr.io/oss-fuzz-base/base-builder-rust
RUN git clone --depth 1 https://github.com/Nil69420/renoir $SRC/renoir
WORKDIR $SRC/renoir
COPY build.sh $SRC/
