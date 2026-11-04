// The emulator's view of what lwIP holds of received data (`emu-census`
// builds call `opendmd_netdiag`; nothing else does, so the linker drops it
// from every other build). Each pbuf counted here is a frame still in one of
// the WiFi driver's receive buffers (no LWIP_L2_TO_L3_COPY), and with all of
// them held the driver drops every frame: see "The receive side" in
// sdkconfig.defaults. Runs on the tcpip thread (tcpip_callback), so the pcb
// lists are not changing under it. Lives in this component because it is the
// project's C component.
#include <stdio.h>
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/queue.h"
#include "lwip/tcpip.h"
#include "lwip/priv/tcp_priv.h"
#include "lwip/priv/sockets_priv.h"
#include "lwip/api.h"
#include "lwip/sockets.h"

static const char *TAG = "netdiag";

static int seg_pbufs(struct tcp_seg *s) {
    int n = 0;
    for (; s; s = s->next) n += pbuf_clen(s->p);
    return n;
}

static int pcb_held(struct tcp_pcb *p) {
    int n = p->refused_data ? pbuf_clen(p->refused_data) : 0;
#if TCP_QUEUE_OOSEQ
    n += seg_pbufs(p->ooseq);
#endif
    return n;
}

static int sock_held(struct lwip_sock *s) {
    struct netconn *c = s->conn;
    int n = s->lastdata.pbuf ? pbuf_clen(s->lastdata.pbuf) : 0;
    if (sys_mbox_valid(&c->recvmbox)) n += uxQueueMessagesWaiting(c->recvmbox->os_mbox);
    return n;
}

static void walk(void *arg) {
    (void)arg;
    int held = 0, open = 0, tw = 0;
    for (struct tcp_pcb *p = tcp_active_pcbs; p; p = p->next) held += pcb_held(p);
    for (struct tcp_pcb *p = tcp_tw_pcbs; p; p = p->next) tw++;
    for (int fd = LWIP_SOCKET_OFFSET; fd < LWIP_SOCKET_OFFSET + CONFIG_LWIP_MAX_SOCKETS; fd++) {
        struct lwip_sock *s = lwip_socket_dbg_get_socket(fd);
        if (!s || !s->conn) continue;
        open++;
        held += sock_held(s);
    }
    if (held) {
        for (struct tcp_pcb *p = tcp_active_pcbs; p; p = p->next) {
            ESP_LOGI(TAG, "pcb :%u st %d wnd %u held %d unacked %d unsent %d rto %d nrtx %d",
                     p->remote_port, p->state, p->rcv_wnd, pcb_held(p), seg_pbufs(p->unacked),
                     seg_pbufs(p->unsent), p->rto, p->nrtx);
        }
        for (int fd = LWIP_SOCKET_OFFSET; fd < LWIP_SOCKET_OFFSET + CONFIG_LWIP_MAX_SOCKETS; fd++) {
            struct lwip_sock *s = lwip_socket_dbg_get_socket(fd);
            if (!s || !s->conn || !sock_held(s)) continue;
            struct netconn *c = s->conn;
            int port = (NETCONNTYPE_GROUP(c->type) == NETCONN_TCP && c->pcb.tcp) ? c->pcb.tcp->remote_port : 0;
            ESP_LOGI(TAG, "fd %d :%d held %d", fd, port, sock_held(s));
        }
    }
    ESP_LOGI(TAG, "sum: %d sockets open, %d TIME_WAIT pcbs, %d rx pbufs held", open, tw, held);
}

void opendmd_netdiag(void) {
    tcpip_callback(walk, NULL);
}

// Sockets with a connection, read from the socket table without lwIP's lock
// (the low watch's snapshot, `emu-census` builds only: a count, no walk).
int opendmd_netdiag_socks(void) {
    int open = 0;
    for (int fd = LWIP_SOCKET_OFFSET; fd < LWIP_SOCKET_OFFSET + CONFIG_LWIP_MAX_SOCKETS; fd++) {
        struct lwip_sock *s = lwip_socket_dbg_get_socket(fd);
        if (s && s->conn) open++;
    }
    return open;
}

// The low watch's snapshot (`emu-census` builds): what lwIP holds at a new
// heap low, counted on the tcpip thread right after the low (posted from the
// low watch, which runs below it on the same core).
volatile int netdiag_snap[6]; // rx pbufs, rx bytes, tx bytes, active pcbs, TIME_WAIT pcbs, sockets
static int seg_bytes(struct tcp_seg *s) {
    int n = 0;
    for (; s; s = s->next) n += s->p ? s->p->tot_len : 0;
    return n;
}
static void snap(void *arg) {
    (void)arg;
    int rx = 0, rxb = 0, txb = 0, act = 0, tw = 0, socks = 0;
    for (struct tcp_pcb *p = tcp_active_pcbs; p; p = p->next) {
        act++;
        rx += pcb_held(p);
        if (p->refused_data) rxb += p->refused_data->tot_len;
#if TCP_QUEUE_OOSEQ
        rxb += seg_bytes(p->ooseq);
#endif
        txb += seg_bytes(p->unacked) + seg_bytes(p->unsent);
    }
    for (struct tcp_pcb *p = tcp_tw_pcbs; p; p = p->next) tw++;
    for (int fd = LWIP_SOCKET_OFFSET; fd < LWIP_SOCKET_OFFSET + CONFIG_LWIP_MAX_SOCKETS; fd++) {
        struct lwip_sock *s = lwip_socket_dbg_get_socket(fd);
        if (!s || !s->conn) continue;
        socks++;
        rx += sock_held(s);
        if (s->lastdata.pbuf) rxb += s->lastdata.pbuf->tot_len;
    }
    netdiag_snap[0] = rx; netdiag_snap[1] = rxb; netdiag_snap[2] = txb;
    netdiag_snap[3] = act; netdiag_snap[4] = tw; netdiag_snap[5] = socks;
}
void opendmd_netdiag_snap(void) {
    tcpip_callback(snap, NULL);
}
