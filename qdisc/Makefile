TARGET = sch_bundle_inbox
#sch_bundle_inbox-objs := sch_bundle_inbox.o # sch_fq_codel.o

ifndef QTYPE
    $(error QTYPE not set. may be one of [fifo|fq_codel|fq|sfq|prio])
endif
ifeq ($(QTYPE), fifo)
	FLAGS = -DQTYPE=1
endif
ifeq ($(QTYPE), fq_codel)
	FLAGS = -DQTYPE=2
endif
ifeq ($(QTYPE), wfq_codel)
	FLAGS = -DQTYPE=6
endif
ifeq ($(QTYPE), fq)
	FLAGS = -DQTYPE=3
endif
ifeq ($(QTYPE), sfq)
	FLAGS = -DQTYPE=4
endif
ifeq ($(QTYPE), prio)
    	FLAGS = -DQTYPE=5
endif

VERBOSE_LOGGING = n
ifeq ($(VERBOSE_LOGGING), y)
	FLAGS += -D__VERBOSE_LOGGING__
endif

EXTRA_CFLAGS += $(FLAGS)

obj-m := $(TARGET).o

all:
	make -C /lib/modules/$(shell uname -r)/build M=$(shell pwd) modules

clean:
	make -C /lib/modules/$(shell uname -r)/build M=$(shell pwd) clean
