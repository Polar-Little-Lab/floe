#import <AppKit/AppKit.h>
#include <assert.h>
#include <stdbool.h>
#include <pthread.h>
extern int branchdesk_install_termination(bool (*callback)(void));
extern bool branchdesk_reply_termination(bool saved);
@interface ExistingTerminationDelegate : NSObject <NSApplicationDelegate>
@end
@implementation ExistingTerminationDelegate
- (NSApplicationTerminateReply)applicationShouldTerminate:(NSApplication *)app {
    (void)app; return NSTerminateCancel;
}
@end
@interface EmptyTerminationDelegate : NSObject <NSApplicationDelegate>
@end
@implementation EmptyTerminationDelegate
@end
static int calls=0;
static bool allow(void) { calls++; return false; }
static void *worker(void *context) {
    (void)context;
    @autoreleasepool {
        assert(![NSThread isMainThread]);
        assert(branchdesk_install_termination(allow)==1);
        assert(!branchdesk_reply_termination(false));
    }
    return NULL;
}
int main(void) {
 @autoreleasepool {
    NSApplication *app=[NSApplication sharedApplication];
    app.delegate=nil;
    assert(branchdesk_install_termination(allow)==1);
    ExistingTerminationDelegate *existing=[ExistingTerminationDelegate new];
    app.delegate=existing;
    assert(branchdesk_install_termination(allow)==2);
    assert([existing applicationShouldTerminate:app]==NSTerminateCancel);
    EmptyTerminationDelegate *empty=[EmptyTerminationDelegate new];
    app.delegate=empty;
    pthread_t thread;
    assert(pthread_create(&thread,NULL,worker,NULL)==0);
    assert(pthread_join(thread,NULL)==0);
    assert(branchdesk_install_termination(allow)==0);
    assert([(id<NSApplicationDelegate>)empty applicationShouldTerminate:app]==NSTerminateNow);
    assert(calls==1);
    assert(branchdesk_install_termination(allow)==2);
    assert(!branchdesk_reply_termination(false));
    app.delegate=nil;
    puts("PASS nil delegate, existing callback preservation, native NSUInteger callback ABI, duplicate refusal, no spurious reply");
 }
 return 0;
}
