USE_FQ_CODEL = n

TARGET = sch_bundle_inbox
#sch_bundle_inbox-objs := sch_bundle_inbox.o # sch_fq_codel.o

ifeq ($(USE_FQ_CODEL),y)
    FLAGS = -D__USE_FQ_CODEL__
endif

EXTRA_CFLAGS += $(FLAGS)

obj-m := $(TARGET).o

all:
	make -C /lib/modules/$(shell uname -r)/build M=$(PWD) modules

clean:
	make -C /lib/modules/$(shell uname -r)/build M=$(PWD) clean
