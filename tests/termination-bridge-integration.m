// Standalone own-process AppKit lifecycle test. No UI automation or user files.
#import <AppKit/AppKit.h>
#include <assert.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
extern int branchdesk_install_termination(bool (*callback)(void));
extern bool branchdesk_reply_termination(bool saved);
static void later(NSTimeInterval seconds, void (^action)(void)) {
    NSTimer *timer=[NSTimer timerWithTimeInterval:seconds repeats:NO block:^(NSTimer *t){(void)t;action();}];
    [[NSRunLoop mainRunLoop] addTimer:timer forMode:NSRunLoopCommonModes];
}
static int requests=0;
static bool cancelled=false;
static bool approval_sent=false;

static bool request_save(void) {
    requests++;
    assert([NSThread isMainThread]);
    const int ordinal=requests;
    assert(ordinal<=2);
    later(0.02,^{
        if(ordinal==1) {
            assert([NSApp.delegate applicationShouldTerminate:NSApp]==NSTerminateLater);
            assert(requests==1);
            assert(branchdesk_reply_termination(false));
            assert(!branchdesk_reply_termination(false));
            cancelled=true;
            puts("CANCELLED first termination; process remains alive");fflush(stdout);
            later(0.02,^{
                [NSApp terminate:nil];
            });
        } else {
            assert(cancelled);
            approval_sent=true;
            puts("APPROVING second termination after asynchronous save");fflush(stdout);
            // Successful approval may terminate the process before this returns.
            (void)branchdesk_reply_termination(true);
        }
    });
    return true;
}
@interface LifecycleDelegate:NSObject<NSApplicationDelegate>
@end
@implementation LifecycleDelegate
-(void)applicationDidFinishLaunching:(NSNotification *)notification {
    (void)notification;
    later(0.01,^{[NSApp terminate:nil];});
}
-(void)applicationWillTerminate:(NSNotification *)notification {
    (void)notification;
    if(requests!=2||!cancelled||!approval_sent){fputs("FAIL premature termination\n",stderr);_Exit(2);}
    puts("PASS asynchronous AppKit termination veto, retry and approval");fflush(stdout);
}
@end
int main(void) {
 @autoreleasepool {
    NSApplication *app=[NSApplication sharedApplication];
    [app setActivationPolicy:NSApplicationActivationPolicyProhibited];
    LifecycleDelegate *delegate=[LifecycleDelegate new];
    app.delegate=delegate;
    assert(branchdesk_install_termination(request_save)==0);
    later(5,^{
        fprintf(stderr,"FAIL timeout, requests=%d\n",requests);_Exit(3);
    });
    [app run];
    fputs("FAIL run returned instead of terminating\n",stderr);
 }
 return 4;
}
