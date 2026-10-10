# The MyBot computer — one per CONVERSATION.
#
# Each conversation gets its own container. Inside it, each bot gets its own
# Linux user, its own home, and its own XFCE desktop on its own X display. Bots
# can hop between desktops, and two containers can be merged only if both sides
# agree (see src/computer/merge.ts).
#
# Desktops are created at RUNTIME, not baked in: the set of bots is not known
# when the image is built. `desktopctl` does that work, and `agentd.py` is the
# in-container handler that the host talks to.

FROM debian:bookworm-slim

ENV DEBIAN_FRONTEND=noninteractive \
    LANG=C.UTF-8 \
    AGENTD_PORT=7000

RUN apt-get update && apt-get install -y --no-install-recommends \
      # desktop
      xfce4 xfce4-terminal xfce4-panel xfdesktop4 \
      dbus-x11 x11-xserver-utils xterm \
      # display + remote view
      xvfb x11vnc novnc websockify \
      # browser
      chromium \
      # agent toolkit
      ca-certificates curl wget jq git unzip nano \
      python3 python3-venv \
      fonts-liberation fonts-noto-color-emoji \
      procps sudo psmisc \
    && rm -rf /var/lib/apt/lists/*

# Xvfb cannot create this as a non-root user; without it the X socket lands
# somewhere the readiness checks never look.
RUN mkdir -p /tmp/.X11-unix && chmod 1777 /tmp/.X11-unix \
    && mkdir -p /workspace /shared /var/lib/mybot \
    && chmod 1777 /workspace /shared

# `agent` owns the daemon and creates per-bot users on demand. It needs sudo for
# useradd only — not a general root shell for the models.
# Every bot user joins the `bots` group. The daemon may run ANY command as a
# member of that group, but only four specific commands as root. That split is
# the point: `agent ALL=(ALL) NOPASSWD: ALL` would let the daemon sudo to root
# and would make the non-root user decorative.
RUN groupadd bots \
    && useradd --create-home --shell /bin/bash --uid 1000 agent \
    && printf '%s\n%s\n' \
       'agent ALL=(root) NOPASSWD: /usr/sbin/useradd, /usr/sbin/userdel, /bin/chown, /bin/mkdir' \
       'agent ALL=(%bots) NOPASSWD: ALL' \
       > /etc/sudoers.d/agent \
    && chmod 0440 /etc/sudoers.d/agent \
    && chown agent:agent /workspace /shared /var/lib/mybot

COPY agentd.py desktopctl.sh cdp-relay.py /usr/local/bin/
RUN chmod +x /usr/local/bin/agentd.py /usr/local/bin/desktopctl.sh /usr/local/bin/cdp-relay.py

# The MyBot wallpaper, as every bot desktop's default (users can still change it).
COPY wallpaper.jpg /usr/share/backgrounds/mybot/wallpaper.jpg
COPY xfce4-desktop.xml /etc/xdg/xfce4/xfconf/xfce-perchannel-xml/xfce4-desktop.xml
# Adwaita has a dark variant, so the panel's default dark mode actually shows;
# the stock "Xfce" theme has none and the panel stayed light grey.
RUN sed -i 's|name="ThemeName" type="string" value="Xfce"|name="ThemeName" type="string" value="Adwaita"|' \
      /etc/xdg/xfce4/xfconf/xfce-perchannel-xml/xsettings.xml \
    && grep -q 'value="Adwaita"' /etc/xdg/xfce4/xfconf/xfce-perchannel-xml/xsettings.xml

USER agent
WORKDIR /workspace

# 7000  agentd control API (host -> container: desktops, files, health)
# 6080+ one noVNC port per bot desktop, allocated in desktop order
# 9223  CDP relay for the primary desktop's Chromium
EXPOSE 7000 6080 6081 6082 6083 9223

HEALTHCHECK --interval=5s --timeout=3s --start-period=25s --retries=12 \
  CMD curl -fsS "http://127.0.0.1:${AGENTD_PORT}/health" >/dev/null || exit 1

ENTRYPOINT ["python3", "/usr/local/bin/agentd.py"]
