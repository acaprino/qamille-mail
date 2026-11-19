# qamille-server per linux/amd64, in due fasi: compilazione con la toolchain Rust,
# poi un'immagine Debian minima con il solo binario. Contesto: la radice del
# repository, perché il server dipende dal crate ../compute (inferenza); i file
# ammessi nel contesto sono in Dockerfile.dockerignore.
#
#   docker build --platform linux/amd64 -t qamille-server -f server/Dockerfile .
#
# Il compose (server/compose.yaml) la costruisce da sé.

# Immagini con versione e digest fissati: un aggiornamento si fa cambiando queste righe.
# Compilazione e immagine finale su Debian 13 (trixie): la libreria statica di ONNX Runtime che `ort` scarica vuole
# glibc 2.38 o più recente e la libstdc++ di GCC 13 o più recente; su bookworm (glibc 2.36, GCC 12) il collegamento
# fallisce. Gli stadi whisper e llama restano su bookworm: i loro programmi girano anche su trixie.
FROM --platform=linux/amd64 rust:1.100.0-trixie@sha256:223cc32ddca8f3ee90c436dbc94b6b5f052ea7f9653c5201e9612f08ad7d21b8 AS build
WORKDIR /src
# server/ e compute/ tranne ciò che esclude Dockerfile.dockerignore (target/, data/,
# .env…): così una cartella server/.cargo/ con i sorgenti delle dipendenze vale anche qui.
COPY . .
WORKDIR /src/server
RUN cargo build --release --locked --jobs 2 --bin qamille-server \
    && strip target/release/qamille-server

# whisper-cli (trascrizione): whisper.cpp compilato da un COMMIT fissato, non dal ramo né da
# un archivio senza hash. Il commit è quello a cui punta il tag v1.9.2
# (git ls-remote https://github.com/ggml-org/whisper.cpp refs/tags/v1.9.2 -> 306c88f...) e
# l'hash è quello dell'archivio di quel commit
# (curl -fsSL https://github.com/ggml-org/whisper.cpp/archive/<commit>.tar.gz | sha256sum).
# La release deve restare quella di qamille_compute::speech::WHISPER_RELEASE (test in
# server/tests/distribution.rs). CPU generica: nessuna istruzione nativa della macchina di build.
FROM --platform=linux/amd64 debian:12.15-slim@sha256:734a72b3d6142a71993b4bc580ca01d8ca687f77a2c422d96a961d3376b179bc AS whisper
ARG WHISPER_RELEASE=v1.9.2
ARG WHISPER_COMMIT=306c88f4d1286aec1bf96e544632897886af5501
ARG WHISPER_SHA256=f3585ebf64df3e41b26c45d93bdb38b423ca1de0bf40cc4b6d320254867a75df
RUN apt-get update     && apt-get install -y --no-install-recommends build-essential cmake ca-certificates curl     && rm -rf /var/lib/apt/lists/*
WORKDIR /w
RUN curl -fsSL -o whisper.tar.gz "https://github.com/ggml-org/whisper.cpp/archive/${WHISPER_COMMIT}.tar.gz"     && echo "${WHISPER_SHA256}  whisper.tar.gz" | sha256sum -c -     && mkdir src && tar -xzf whisper.tar.gz --strip-components=1 -C src
WORKDIR /w/src
RUN cmake -B build -DCMAKE_BUILD_TYPE=Release -DGGML_NATIVE=OFF -DGGML_OPENMP=OFF         -DBUILD_SHARED_LIBS=OFF -DWHISPER_BUILD_TESTS=OFF -DWHISPER_BUILD_EXAMPLES=ON     && cmake --build build --config Release --target whisper-cli -j 2     && install -D build/bin/whisper-cli /usr/local/bin/whisper-cli     && strip /usr/local/bin/whisper-cli     && install -D LICENSE /usr/share/licenses/whisper.cpp/LICENSE

# llama-server (assistente interno, spec L8): llama.cpp compilato da un COMMIT fissato, non dal ramo.
# Il commit è quello a cui punta il tag LLAMA_TAG (git ls-remote https://github.com/ggml-org/llama.cpp
# "refs/tags/b12081*") e si scarica con git per identificatore: il contenuto è fissato dal commit di
# 40 cifre, che git controlla da sé, e non dipende dalla stabilità degli archivi .tar.gz di GitHub.
# LLAMA_SHA256 è l'hash dell'archivio .tar.gz di quel commit (curl -fsSL
# https://github.com/ggml-org/llama.cpp/archive/<commit>.tar.gz | sha256sum): resta qui perché chi
# vuole controllare a mano lo trovi, ed è lo stesso di qamille_compute::llama::SOURCE_SHA256.
# Tag, commit e hash devono restare quelli di qamille_compute::llama (test in server/tests/distribution.rs;
# per cambiarli si usa compute/tools/llama-lookup.mjs). CPU generica, senza OpenMP e senza il
# download dell'interfaccia web a build, né HTTPS: il server non scarica nulla da sé.
FROM --platform=linux/amd64 debian:12.15-slim@sha256:734a72b3d6142a71993b4bc580ca01d8ca687f77a2c422d96a961d3376b179bc AS llama
ARG LLAMA_TAG=b12081
ARG LLAMA_COMMIT=8fc0376721e9662d498a5706d7ee8e7574ce99f4
ARG LLAMA_SHA256=75f4a0e0663d172ee75744179689139a8ed933bc169d1d1264fa701f802ea66a
RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential cmake ca-certificates git \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /l/src
RUN git init -q . \
    && git remote add origin https://github.com/ggml-org/llama.cpp.git \
    && git fetch -q --depth 1 origin "${LLAMA_COMMIT}" \
    && git checkout -q FETCH_HEAD \
    && test "$(git rev-parse HEAD)" = "${LLAMA_COMMIT}"
RUN cmake -B build -DCMAKE_BUILD_TYPE=Release -DGGML_NATIVE=OFF -DGGML_OPENMP=OFF \
        -DBUILD_SHARED_LIBS=OFF -DLLAMA_OPENSSL=OFF -DLLAMA_USE_PREBUILT_UI=OFF -DLLAMA_BUILD_UI=OFF \
        -DLLAMA_BUILD_TESTS=OFF -DLLAMA_BUILD_EXAMPLES=OFF -DLLAMA_BUILD_APP=OFF \
        -DLLAMA_BUILD_TOOLS=ON -DLLAMA_BUILD_SERVER=ON \
        -DLLAMA_BUILD_NUMBER="${LLAMA_TAG#b}" -DLLAMA_BUILD_COMMIT="${LLAMA_COMMIT}" \
    && cmake --build build --config Release --target llama-server -j 2 \
    && install -D build/bin/llama-server /usr/local/bin/llama-server \
    && strip /usr/local/bin/llama-server \
    && install -D LICENSE /usr/share/licenses/llama.cpp/LICENSE
# Le opzioni che il server passa al figlio (qamille_server::llm_builtin::REQUIRED_FLAGS): se una
# sparisce da una versione nuova, la build si ferma qui e non a runtime.
RUN llama-server --help > /tmp/llama-help.txt 2>&1 || true; \
    for flag in --model --host --port --ctx-size --parallel --cache-ram --no-slots --jinja --no-webui --log-verbosity; do \
        grep -q -F -e "$flag" /tmp/llama-help.txt || { echo "llama-server does not know $flag" >&2; exit 1; }; \
    done

FROM --platform=linux/amd64 debian:13.7-slim@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2
ARG QAMILLE_VERSION=dev
ARG QAMILLE_SOURCE=https://github.com/acaprino/qamille-mail
# licenses: il server è MIT (LICENSE); whisper.cpp e llama.cpp (MIT) sono in /usr/share/licenses.
LABEL org.opencontainers.image.title="qamille-server"       org.opencontainers.image.version="${QAMILLE_VERSION}"       org.opencontainers.image.source="${QAMILLE_SOURCE}"       org.opencontainers.image.licenses="MIT"
# Utente senza privilegi; /data è il volume con server.db (e server.toml, se c'è).
RUN groupadd --system --gid 10001 qamille \
    && useradd --system --uid 10001 --gid qamille --home-dir /data --shell /usr/sbin/nologin qamille \
    && mkdir -p /data \
    && chown qamille:qamille /data \
    && chmod 0700 /data
COPY --from=build /src/server/target/release/qamille-server /usr/local/bin/qamille-server
COPY --from=whisper /usr/local/bin/whisper-cli /usr/local/bin/whisper-cli
COPY --from=whisper /usr/share/licenses/whisper.cpp/LICENSE /usr/share/licenses/whisper.cpp/LICENSE
COPY --from=llama /usr/local/bin/llama-server /usr/local/bin/llama-server
COPY --from=llama /usr/share/licenses/llama.cpp/LICENSE /usr/share/licenses/llama.cpp/LICENSE
# Una libreria che manca nell'immagine minima ferma la build qui, non l'avvio del server.
RUN if ldd /usr/local/bin/qamille-server /usr/local/bin/whisper-cli /usr/local/bin/llama-server | grep -F "not found"; then exit 1; fi
ENV QAMILLE_DATA_DIR=/data \
    QAMILLE_BIND=0.0.0.0:8787
USER qamille:qamille
WORKDIR /data
VOLUME ["/data"]
EXPOSE 8787 8788
STOPSIGNAL SIGTERM
ENTRYPOINT ["qamille-server"]
CMD ["serve"]
