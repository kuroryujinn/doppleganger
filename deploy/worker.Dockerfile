FROM scratch
ARG ENGINE_SHA256
LABEL org.doppelganger.engine-sha256=$ENGINE_SHA256
LABEL org.doppelganger.protocol="1"
COPY rootfs/ /
USER 10001:10001
WORKDIR /work
ENTRYPOINT ["/opt/dg-sandbox"]
