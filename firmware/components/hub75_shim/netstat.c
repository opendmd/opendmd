// The network side's view of lwIP for `/api/status` `net` (every build;
// `src/netstat.rs`). Each walk runs on the tcpip thread (tcpip_callback),
// so the pcb lists are not changing under it; the network worker posts one
// a second, and the counts are read whenever.
//
// And the close of a connection whose client has gone: `opendmd_close_dead`
// aborts (RST, the pcb freed at once) a connection in CLOSE_WAIT that still
// has data the client never acknowledged; a plain close would keep the pcb,
// that data and its retransmissions (up to TCP_MAXRTX, minutes) for nobody.
#include <stdint.h>
#include "freertos/FreeRTOS.h"
#include "freertos/queue.h"
#include "lwip/tcpip.h"
#include "lwip/priv/tcp_priv.h"
#include "lwip/priv/sockets_priv.h"
#include "lwip/api.h"
#include "lwip/sockets.h"

// pcbs, TIME_WAIT pcbs, rx pbufs held, tx bytes queued, most retransmissions
// of one segment (nrtx) among the pcbs, walks done
volatile int opendmd_netstat[6];
volatile int opendmd_netstat_aborted;
// Connections waiting for accept() at the walk (the most over the listening
// pcbs' accepts_pending: handshaking, or established and not taken yet);
// lwIP drops a SYN once that reaches the listen backlog (net-stall).
volatile int opendmd_netstat_backlog;

static int seg_pbufs(struct tcp_seg *s) {
    int n = 0;
    for (; s; s = s->next) n += pbuf_clen(s->p);
    return n;
}
static int seg_bytes(struct tcp_seg *s) {
    int n = 0;
    for (; s; s = s->next) n += s->p ? s->p->tot_len : 0;
    return n;
}

static void walk(void *arg) {
    (void)arg;
    int pcbs = 0, tw = 0, rx = 0, tx = 0, rtx = 0, backlog = 0;
    for (struct tcp_pcb *p = tcp_active_pcbs; p; p = p->next) {
        pcbs++;
        // Frames waiting in the pcb's netconn mailbox, accepted or not. Only
        // through the pcb: this thread clears callback_arg when a netconn
        // lets go of it, and the netconn is freed only after that, so it is
        // alive here. A socket's own fields (its conn, its half-read pbuf)
        // belong to the task using it and can be freed under this walk.
        if (p->state != SYN_RCVD && p->callback_arg) {
            struct netconn *c = (struct netconn *)p->callback_arg;
            if (sys_mbox_valid(&c->recvmbox)) rx += uxQueueMessagesWaiting(c->recvmbox->os_mbox);
        }
        if (p->refused_data) rx += pbuf_clen(p->refused_data);
#if TCP_QUEUE_OOSEQ
        rx += seg_pbufs(p->ooseq);
#endif
        tx += seg_bytes(p->unacked) + seg_bytes(p->unsent);
        if (p->nrtx > rtx) rtx = p->nrtx;
    }
    for (struct tcp_pcb *p = tcp_tw_pcbs; p; p = p->next) tw++;
#if TCP_LISTEN_BACKLOG
    for (struct tcp_pcb_listen *l = tcp_listen_pcbs.listen_pcbs; l; l = l->next)
        if (l->accepts_pending > backlog) backlog = l->accepts_pending;
#endif
    opendmd_netstat[0] = pcbs;
    opendmd_netstat[1] = tw;
    opendmd_netstat[2] = rx;
    opendmd_netstat[3] = tx;
    opendmd_netstat[4] = rtx;
    opendmd_netstat_backlog = backlog;
    opendmd_netstat[5]++;
}

void opendmd_netstat_post(void) {
    tcpip_callback(walk, NULL);
}

static void close_dead(void *arg) {
    int fd = (int)(intptr_t)arg;
    struct lwip_sock *s = lwip_socket_dbg_get_socket(fd);
    if (!s || !s->conn || NETCONNTYPE_GROUP(s->conn->type) != NETCONN_TCP) return;
    struct tcp_pcb *p = s->conn->pcb.tcp;
    if (p && p->state == CLOSE_WAIT && (p->unacked || p->unsent)) {
        opendmd_netstat_aborted++;
        tcp_abort(p); // the netconn's error callback lets go of the pcb
    }
}

// Before `close(fd)`, on the task closing it: posted ahead of the close's own
// message to the tcpip thread, which handles them in order.
void opendmd_close_dead(int fd) {
    tcpip_callback(close_dead, (void *)(intptr_t)fd);
}
