import bpy
for name in ['Material.001','rastMat']:
    m=bpy.data.materials.get(name)
    print('===',name)
    for n in m.node_tree.nodes:
        print(' NODE',n.type,n.name)
        for i in n.inputs:
            if i.is_linked:
                for l in i.links:
                    print('   in',i.name,'<-',l.from_node.type,l.from_node.name,l.from_socket.name)
        for o in n.outputs:
            for l in o.links:
                print('   out',o.name,'->',l.to_node.type,l.to_node.name,l.to_socket.name)
