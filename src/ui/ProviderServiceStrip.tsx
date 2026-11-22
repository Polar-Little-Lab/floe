const services=[
 {name:'GitHub',image:'github-logo.png',available:true},
 {name:'GitHub Enterprise Server',image:'github-logo.png',available:false},
 {name:'GitLab',image:'gitlab-color.png',available:false},
 {name:'GitLab Self-Managed',image:'gitlab-color.png',available:false},
 {name:'Bitbucket',image:'bitbucket-color.png',available:false},
 {name:'Azure DevOps',image:'azuredevops-color.svg',available:false},
];
// Reference service order; visual assets retain vendor provenance.json entries.
export function ProviderServiceStrip(){return <div className="provider-service-strip" role="group" aria-label="Hosting service">{services.map(service=><button key={service.name} type="button" aria-pressed={service.available} disabled={!service.available} title={service.available?'GitHub selected':`${service.name} is not implemented yet`}><img src={`/vendor/gitkraken/src/images/${service.image}`} alt=""/><span>{service.name}</span>{!service.available&&<span className="provider-service-unavailable">Unavailable</span>}</button>)}</div>;}
