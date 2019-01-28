/*
 * net/sched/sch_prio.c	Simple 3-band priority "scheduler".
 *
 *		This program is free software; you can redistribute it and/or
 *		modify it under the terms of the GNU General Public License
 *		as published by the Free Software Foundation; either version
 *		2 of the License, or (at your option) any later version.
 *
 * Authors:	Alexey Kuznetsov, <kuznet@ms2.inr.ac.ru>
 * Fixes:       19990609: J Hadi Salim <hadi@nortelnetworks.com>:
 *              Init --  EINVAL when opt undefined
 */

#include <linux/module.h>
#include <linux/slab.h>
#include <linux/types.h>
#include <linux/kernel.h>
#include <linux/string.h>
#include <linux/errno.h>
#include <linux/skbuff.h>
#include <net/netlink.h>
#include <net/pkt_sched.h>
#include <net/pkt_cls.h>
#include <linux/byteorder/generic.h>


#define NUM_QUEUES 2

struct prio_sched_data {
	int bands;
	struct tcf_proto __rcu *filter_list;
	struct tcf_block *block;
	u8  prio2band[TC_PRIO_MAX+1];
	struct Qdisc *queues[NUM_QUEUES];
};

static struct Qdisc *
prio_classify(struct sk_buff *skb, struct Qdisc *sch, int *qerr)
{
	struct prio_sched_data *q = qdisc_priv(sch);
  u32 band = 1;

  struct tcphdr *tcp_header;
  tcp_header = (struct tcphdr *)skb_transport_header(skb);
  u32 sport = ntohs(tcp_header->source);
  if (sport >= 5000 && sport <= 6000) {
    band = 0;
  }
  
	*qerr = NET_XMIT_SUCCESS | __NET_XMIT_BYPASS;
  return q->queues[band];
  /*
	struct tcf_result res;
	struct tcf_proto *fl;
	int err;

	*qerr = NET_XMIT_SUCCESS | __NET_XMIT_BYPASS;
	if (TC_H_MAJ(skb->priority) != sch->handle) {
    pr_info("boo\n");
		fl = rcu_dereference_bh(q->filter_list);
		err = tcf_classify(skb, fl, &res, false);
#ifdef CONFIG_NET_CLS_ACT
		switch (err) {
		case TC_ACT_STOLEN:
		case TC_ACT_QUEUED:
		case TC_ACT_TRAP:
			*qerr = NET_XMIT_SUCCESS | __NET_XMIT_STOLEN;
			// fall through 
		case TC_ACT_SHOT:
			return NULL;
		}
#endif

		if (!fl || err < 0) {
			if (TC_H_MAJ(band))
				band = 0;
			return q->queues[q->prio2band[band & TC_PRIO_MAX]];
		}
		band = res.classid;
	}

	band = TC_H_MIN(band) - 1;
	if (band >= q->bands)
		return q->queues[q->prio2band[0]];

	return q->queues[band];
  */
}

static int
prio_enqueue(struct sk_buff *skb, struct Qdisc *sch, struct sk_buff **to_free)
{
	struct Qdisc *qdisc;
	struct prio_sched_data *q = qdisc_priv(sch);
	int ret;

#ifdef __VERBOSE_LOGGING__
  pr_info("[prio] qlen %d %d", q->queues[0]->q.qlen, q->queues[1]->q.qlen);
#endif

	qdisc = prio_classify(skb, sch, &ret);
#ifdef CONFIG_NET_CLS_ACT
	if (qdisc == NULL) {

		if (ret & __NET_XMIT_BYPASS)
			qdisc_qstats_drop(sch);
		__qdisc_drop(skb, to_free);
		return ret;
	}
#endif

	ret = qdisc_enqueue(skb, qdisc, to_free);
	if (ret == NET_XMIT_SUCCESS) {
		qdisc_qstats_backlog_inc(sch, skb);
		sch->q.qlen++;
		return NET_XMIT_SUCCESS;
	}
	if (net_xmit_drop_count(ret))
		qdisc_qstats_drop(sch);
	return ret;
}

static struct sk_buff *prio_peek(struct Qdisc *sch)
{
	struct prio_sched_data *q = qdisc_priv(sch);
	int prio;

	for (prio = 0; prio < NUM_QUEUES; prio++) {
		struct Qdisc *qdisc = q->queues[prio];
    if (!qdisc) {
      pr_info("prio_peek: found empty qdisc %d\n", prio);
      return NULL;
    }
		struct sk_buff *skb = qdisc->ops->peek(qdisc);
		if (skb)
			return skb;
	}
	return NULL;
}

static struct sk_buff *prio_dequeue(struct Qdisc *sch)
{
	struct prio_sched_data *q = qdisc_priv(sch);
	int prio;

	for (prio = 0; prio < NUM_QUEUES; prio++) {
		struct Qdisc *qdisc = q->queues[prio];
    if (!qdisc) {
      pr_info("prio_dequeue: found empty qdisc %d\n", prio);
      return NULL;
    }
		struct sk_buff *skb = qdisc_dequeue_peeked(qdisc);
		if (skb) {
			qdisc_bstats_update(sch, skb);
			qdisc_qstats_backlog_dec(sch, skb);
			sch->q.qlen--;
#ifdef __VERBOSE_LOGGING__
      pr_info("[prio] qlen %d %d", q->queues[0]->q.qlen, q->queues[1]->q.qlen);
#endif
			return skb;
		}
	}
	return NULL;

}

static void
prio_reset(struct Qdisc *sch)
{
	int prio;
	struct prio_sched_data *q = qdisc_priv(sch);

  for (prio = 0; prio < NUM_QUEUES; prio++) {
    if (q->queues[prio]) {
      qdisc_reset(q->queues[prio]);
    } else {
      pr_info("prio_reset: queue %d does not exist yet\n", prio);
    }
  }
	sch->qstats.backlog = 0;
	sch->q.qlen = 0;
}

static void
prio_destroy(struct Qdisc *sch)
{
	int prio;
	struct prio_sched_data *q = qdisc_priv(sch);

	tcf_block_put(q->block);
  for (prio = 0; prio < NUM_QUEUES; prio++) {
    if (q->queues[prio]) {
      qdisc_destroy(q->queues[prio]);
    } else {
      pr_info("prio_destroy: queue %d does not exist yet\n", prio);
    }
  }
}

static int prio_simple(struct Qdisc *sch) {
	struct prio_sched_data *q = qdisc_priv(sch);
	struct Qdisc *queues[NUM_QUEUES];
  int i=0;

  for (i=0; i<NUM_QUEUES; i++) {
    queues[i] = qdisc_create_dflt(sch->dev_queue, &pfifo_qdisc_ops, TC_H_MAKE(sch->handle, i+1));
    if (!queues[i]) {
      return -ENOMEM;
    }
  }

  sch_tree_lock(sch);
  for (i=0; i<NUM_QUEUES; i++) {
		q->queues[i] = queues[i];
    if (q->queues[i] != &noop_qdisc) {
      qdisc_hash_add(q->queues[i], true);
    }
  }
  sch_tree_unlock(sch);
  return 0;
}

static int prio_tune(struct Qdisc *sch, struct nlattr *opt)
{
  return prio_simple(sch);
}

static int prio_init(struct Qdisc *sch, struct nlattr *opt)
{
	struct prio_sched_data *q = qdisc_priv(sch);
	int err;

	err = tcf_block_get(&q->block, &q->filter_list, sch);
	if (err)
		return err;

  return prio_simple(sch);
}

static int prio_dump(struct Qdisc *sch, struct sk_buff *skb)
{
	struct prio_sched_data *q = qdisc_priv(sch);

  pr_info("prio_dump: not implemented\n");
  return -1;
  /*
	unsigned char *b = skb_tail_pointer(skb);
	struct tc_prio_qopt opt;

	opt.bands = q->bands;
	memcpy(&opt.priomap, q->prio2band, TC_PRIO_MAX + 1);

	if (nla_put(skb, TCA_OPTIONS, sizeof(opt), &opt))
		goto nla_put_failure;

	return skb->len;

nla_put_failure:
	nlmsg_trim(skb, b);
	return -1;
  */
}

static int prio_graft(struct Qdisc *sch, unsigned long arg, struct Qdisc *new,
		      struct Qdisc **old)
{
	struct prio_sched_data *q = qdisc_priv(sch);
	unsigned long band = arg - 1;
  pr_info("prio_graft: not implemented\n");
  return 0;
  /*

	if (new == NULL)
		new = &noop_qdisc;

	*old = qdisc_replace(sch, new, &q->queues[band]);
	return 0;
  */
}

static struct Qdisc *
prio_leaf(struct Qdisc *sch, unsigned long arg)
{
	struct prio_sched_data *q = qdisc_priv(sch);
	unsigned long band = arg - 1;
  pr_info("prio_leaf: not implemented\n");
  if (band < 0 || band > 1) {
    band = 1;
  }
	return q->queues[band];
}

static unsigned long prio_find(struct Qdisc *sch, u32 classid)
{
	struct prio_sched_data *q = qdisc_priv(sch);
	unsigned long band = TC_H_MIN(classid);
  pr_info("prio_find: not implemented\n");
  
	if (band - 1 >= q->bands)
		return 0;
	return band;
}

static unsigned long prio_bind(struct Qdisc *sch, unsigned long parent, u32 classid)
{
	return prio_find(sch, classid);
}


static void prio_unbind(struct Qdisc *q, unsigned long cl)
{
}

static int prio_dump_class(struct Qdisc *sch, unsigned long cl, struct sk_buff *skb,
			   struct tcmsg *tcm)
{
	struct prio_sched_data *q = qdisc_priv(sch);

  pr_info("dump_class queue 0\n");
	tcm->tcm_handle |= TC_H_MIN(cl);
	tcm->tcm_info = q->queues[0]->handle;
	return 0;
}

static int prio_dump_class_stats(struct Qdisc *sch, unsigned long cl,
				 struct gnet_dump *d)
{
	struct prio_sched_data *q = qdisc_priv(sch);
	struct Qdisc *cl_q;

	cl_q = q->queues[0];
	if (gnet_stats_copy_basic(qdisc_root_sleeping_running(sch),
				  d, NULL, &cl_q->bstats) < 0 ||
	    gnet_stats_copy_queue(d, NULL, &cl_q->qstats, cl_q->q.qlen) < 0)
		return -1;

	return 0;
}

static void prio_walk(struct Qdisc *sch, struct qdisc_walker *arg)
{
	struct prio_sched_data *q = qdisc_priv(sch);
	int prio;
  pr_info("prio_walk\n");

	if (arg->stop)
		return;

	for (prio = 0; prio < NUM_QUEUES; prio++) {
		if (arg->count < arg->skip) {
			arg->count++;
			continue;
		}
		if (arg->fn(sch, prio + 1, arg) < 0) {
			arg->stop = 1;
			break;
		}
		arg->count++;
	}
}

static struct tcf_block *prio_tcf_block(struct Qdisc *sch, unsigned long cl)
{
	struct prio_sched_data *q = qdisc_priv(sch);

	if (cl)
		return NULL;
	return q->block;
}

static const struct Qdisc_class_ops prio_class_ops = {
	.graft		=	prio_graft,
	.leaf		=	prio_leaf,
	.find		=	prio_find,
	.walk		=	prio_walk,
	.tcf_block	=	prio_tcf_block,
	.bind_tcf	=	prio_bind,
	.unbind_tcf	=	prio_unbind,
	.dump		=	prio_dump_class,
	.dump_stats	=	prio_dump_class_stats,
};

static struct Qdisc_ops prio_qdisc_ops __read_mostly = {
	.next		=	NULL,
	.cl_ops		=	&prio_class_ops,
	.id		=	"prio",
	.priv_size	=	sizeof(struct prio_sched_data),
	.enqueue	=	prio_enqueue,
	.dequeue	=	prio_dequeue,
	.peek		=	prio_peek,
	.init		=	prio_init,
	.reset		=	prio_reset,
	.destroy	=	prio_destroy,
	.change		=	prio_tune,
	.dump		=	prio_dump,
	.owner		=	THIS_MODULE,
};
